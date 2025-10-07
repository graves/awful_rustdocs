mod defaults;

mod cli;
mod error;
mod grep;
mod harvest;
mod logging;
mod model;
mod patch;
mod pipeline;
mod prompt;
mod regexes;
mod runner;
mod sanitize;
mod util;

use crate::cli::{Cli, Command, config_root, run_init};
use crate::error::{Error, Result};
use crate::harvest::run_nushell_harvest;
use crate::patch::patch_files_with_docs;

use awful_aj::config::{AwfulJadeConfig, load_config};
use awful_aj::template::{self, ChatTemplate};
use clap::Parser;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, prelude::*};

use std::path::Path;
use std::path::PathBuf;

/// Initializes global tracing with a configured filter and formatted output layer.
///
/// Sets up the global tracing subscriber using environment-based filtering (defaulting to "info")
/// if not specified. The output is formatted with target and level visibility enabled, and
/// compact formatting is applied to reduce verbosity. This function is called during startup
/// to ensure structured logging is available throughout the application.
///
/// # Notes
/// - The filter is derived from the `RUST_LOG` environment variable or defaults to `"info"`.
/// - The formatter includes target names and log levels, with compact output to minimize overhead.
/// - This function is private and intended for internal use only.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_level(true)
                .compact(),
        )
        .init();
}

/// Entry point for the Awful Jade CLI application.
///
/// Parses command-line arguments and routes execution to either initialization (`Init`) or runtime processing (`Run`).
/// In `Run` mode, it loads the configuration, templates, and performs AST harvesting via Nushell, then runs LLM-powered
/// documentation generation. The generated documentation is serialized to `target/llm_rustdocs/docs.json` and optionally
/// patched into source files. Logging and error handling are integrated throughout.
///
/// # Parameters
/// - `cli`: Parsed [`Cli`] from command-line arguments.
///
/// # Returns
/// - `Ok(())` on successful execution.
/// - `Err(Error)` if any step fails, including config loading, template parsing, harvesting, generation, or I/O operations.
///
/// # Errors
/// - `Error::External` when loading config, templates, or during Nushell harvesting.
/// - `Error::Io` when creating directories or writing files.
/// - `Error::Json` when serializing generated results to JSON.
/// - Any other errors from internal components like `pipeline::run_generation` or `template::load_template`.
///
/// # Notes
/// - The `config` path is resolved relative to the config root if not absolute.
/// - If `--write` is not specified, generated docs are not written to source files.
/// - Default targets are set to the current directory if none are provided.
/// - Logging is enabled with debug and info levels throughout.
#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    debug!(
        "logger initialized; args={:?}",
        std::env::args().collect::<Vec<_>>()
    );

    let cli = Cli::parse();
    debug!("parsed CLI: {:?}", std::env::args().collect::<Vec<_>>());

    match cli.cmd {
        Command::Init { force, dry_run } => {
            info!(force, dry_run, "running init");
            if dry_run {
                warn!("init in dry-run mode: printing planned paths only");
            }
            run_init(force, dry_run)?;
            info!("init completed");
            Ok(())
        }
        Command::Run(opts) => {
            info!("run: starting");
            debug!(?opts, "effective options");

            // Resolve config path
            let cfg_path: String = if Path::new(&opts.config).is_absolute() {
                opts.config.clone()
            } else {
                let root = config_root()?;
                debug!(root=?root, file=?opts.config, "resolved config root");
                root.join(&opts.config).to_string_lossy().into_owned()
            };
            info!(cfg_path = %cfg_path, "loading Awful Jade config");

            // Load AJ config
            let mut cfg: AwfulJadeConfig = load_config(&cfg_path).map_err(|e| {
                error!(error=%e, cfg_path=%cfg_path, "failed to load Awful Jade config");
                Error::External {
                    context: "Failed to load Awful Jade config",
                    message: format!("{}: {}", cfg_path, e),
                }
            })?;

            if let Some(name) = &opts.session {
                info!(session = %name, "ensuring AJ conversation + session config");
                cfg.ensure_conversation_and_config(name)
                    .await
                    .map_err(|e| {
                        error!(error=%e, session=%name, "ensure_conversation_and_config failed");
                        Error::External {
                            context: "ensure_conversation_and_config failed",
                            message: e.to_string(),
                        }
                    })?;
            }

            // Load templates
            info!(fn_template=%opts.fn_template, "loading function template");
            let tpl_fn: ChatTemplate = template::load_template(&opts.fn_template)
                .await
                .map_err(|e| {
                    error!(error=%e, template=%opts.fn_template, "failed to load function template");
                    Error::External {
                        context: "Failed to load function template",
                        message: format!("'{}': {}", opts.fn_template, e),
                    }
                })?;

            info!(struct_template=%opts.struct_template, "loading struct template");
            let tpl_struct: ChatTemplate = template::load_template(&opts.struct_template)
                .await
                .map_err(|e| {
                    error!(error=%e, template=%opts.struct_template, "failed to load struct template");
                    Error::External {
                        context: "Failed to load struct template",
                        message: format!("'{}': {}", opts.struct_template, e),
                    }
                })?;

            // Build context
            let ctx = pipeline::Ctx {
                cfg,
                tpl_fn,
                tpl_struct,
                opts: opts.clone(),
            };

            // Targets
            let targets: Vec<PathBuf> = if ctx.opts.targets.is_empty() {
                info!("no targets provided; defaulting to current directory '.'");
                vec![PathBuf::from(".")]
            } else {
                info!(count = ctx.opts.targets.len(), "received explicit targets");
                ctx.opts.targets.clone()
            };
            debug!(?targets, "targets to analyze");

            // Harvest
            info!("harvesting AST rows via Nushell");
            let rows = run_nushell_harvest(&ctx.opts.script, &targets)?;
            info!(rows = rows.len(), "harvest completed");

            // Generate
            info!("starting LLM doc generation");
            let all_results = pipeline::run_generation(&ctx, rows).await?;
            info!(generated = all_results.len(), "generation finished");

            // Persist results
            let out_dir = PathBuf::from("target/llm_rustdocs");
            debug!(dir=?out_dir, "ensuring output directory");
            std::fs::create_dir_all(&out_dir).map_err(|e| {
                error!(error=%e, ?out_dir, "failed to create output directory");
                Error::Io {
                    path: Some(out_dir.clone()),
                    source: e,
                }
            })?;
            let out_json = out_dir.join("docs.json");
            info!(file=%out_json.to_string_lossy(), "writing docs.json");
            std::fs::write(
                &out_json,
                serde_json::to_vec_pretty(&all_results).map_err(|e| {
                    error!(error=%e, "failed to serialize docs.json");
                    Error::Json {
                        context: "serialize docs.json",
                        source: e,
                    }
                })?,
            )
            .map_err(|e| {
                error!(error=%e, file=%out_json.to_string_lossy(), "failed to write docs.json");
                Error::Io {
                    path: Some(out_json.clone()),
                    source: e,
                }
            })?;
            info!(file=%out_json.to_string_lossy(), "wrote docs.json");

            // Patch source files
            if ctx.opts.write {
                info!("patching source files with generated rustdoc");
                patch_files_with_docs(&all_results, ctx.opts.overwrite)?;
                info!("patching complete");
            } else {
                warn!("--write not set; skipping patching of source files");
            }

            info!("run: completed successfully");
            Ok(())
        }
    }
}
