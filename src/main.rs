mod defaults;

mod error;
mod model;
mod regexes;
mod runner;
mod grep;
mod harvest;
mod patch;
mod prompt;
mod sanitize;
mod util;
mod pipeline;
mod cli;

use crate::cli::{Cli, Command, run_init, config_root};
use crate::error::{Error, Result};
use crate::harvest::run_nushell_harvest;
use crate::patch::patch_files_with_docs;

use awful_aj::config::{load_config, AwfulJadeConfig};
use awful_aj::template::{self, ChatTemplate};

use clap::Parser;
use std::path::Path;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Command::Init { force, dry_run } => {
            run_init(force, dry_run)?;
            Ok(())
        }
        Command::Run(opts) => {
            // Resolve config path
            let cfg_path: String = if Path::new(&opts.config).is_absolute() {
                opts.config.clone()
            } else {
                config_root()?
                    .join(&opts.config)
                    .to_string_lossy()
                    .into_owned()
            };

            // Load AJ config
            let mut cfg: AwfulJadeConfig = load_config(&cfg_path).map_err(|e| Error::External {
                context: "Failed to load Awful Jade config",
                message: format!("{}: {}", cfg_path, e),
            })?;

            if let Some(name) = &opts.session {
                cfg.ensure_conversation_and_config(name)
                    .await
                    .map_err(|e| Error::External {
                        context: "ensure_conversation_and_config failed",
                        message: e.to_string(),
                    })?;
            }

            // Load templates
            let tpl_fn: ChatTemplate =
                template::load_template(&opts.fn_template)
                    .await
                    .map_err(|e| Error::External {
                        context: "Failed to load function template",
                        message: format!("'{}': {}", opts.fn_template, e),
                    })?;
            let tpl_struct: ChatTemplate = template::load_template(&opts.struct_template)
                .await
                .map_err(|e| Error::External {
                    context: "Failed to load struct template",
                    message: format!("'{}': {}", opts.struct_template, e),
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
                vec![PathBuf::from(".")]
            } else {
                ctx.opts.targets.clone()
            };

            // Harvest
            let rows = run_nushell_harvest(&ctx.opts.script, &targets)?;

            // Generate
            let all_results = pipeline::run_generation(&ctx, rows).await?;

            // Persist results
            let out_dir = PathBuf::from("target/llm_rustdocs");
            std::fs::create_dir_all(&out_dir).map_err(|e| Error::Io {
                path: Some(out_dir.clone()),
                source: e,
            })?;
            let out_json = out_dir.join("docs.json");
            std::fs::write(
                &out_json,
                serde_json::to_vec_pretty(&all_results).map_err(|e| Error::Json {
                    context: "serialize docs.json",
                    source: e,
                })?,
            )
            .map_err(|e| Error::Io {
                path: Some(out_json.clone()),
                source: e,
            })?;
            eprintln!("Wrote {}", out_json.to_string_lossy());

            // Patch source files
            if ctx.opts.write {
                patch_files_with_docs(&all_results, ctx.opts.overwrite)?;
            }

            Ok(())
        }
    }
}
