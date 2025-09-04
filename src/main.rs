mod defaults;

use crate::defaults::{DEFAULT_CONFIG_YAML, DEFAULT_RUSTDOC_FN_YAML, DEFAULT_RUSTDOC_STRUCT_YAML};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};
use directories::ProjectDirs;
use itertools::Itertools;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcCommand, Stdio};
use tempfile::NamedTempFile;

// Awful Jade
use awful_aj::api;
use awful_aj::config::{AwfulJadeConfig, load_config};
use awful_aj::template::{self, ChatTemplate};

#[derive(Parser, Debug)]
#[command(
    name = "awful_rustdocs",
    about = "Generate rustdocs for functions and structs using Awful Jade + rust_ast.nu"
)]
/// Represents a CLI command structure with a subcommand.
struct Cli {
    /// The subcommand command, used to invoke the appropriate functionality.
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize default config & templates in the OS config directory.
    Init {
        /// Overwrite files if they already exist.
        #[arg(long, action=ArgAction::SetTrue)]
        force: bool,
        /// Print the paths that were (or would be) written.
        #[arg(long, action=ArgAction::SetTrue)]
        dry_run: bool,
    },

    /// (default) Generate rustdocs using current options.
    Run(GenerateOpts),
}

/// Options for configuring rustdoc generation in Awful Jade.
#[derive(Debug, clap::Args)]
struct GenerateOpts {
    /// Path to your rust_ast.nu (the Nu script you shared)
    #[arg(long, default_value = "rust_ast.nu")]
    script: Utf8PathBuf,

    /// Paths (files/dirs) to analyze (default: ".")
    #[arg()]
    targets: Vec<Utf8PathBuf>,

    /// Write docs directly into source files (prepending ///)
    #[arg(long, action=ArgAction::SetTrue)]
    write: bool,

    /// Overwrite existing rustdoc if present (default: false; only fills missing)
    #[arg(long, action=ArgAction::SetTrue)]
    overwrite: bool,

    /// Session name for Awful Jade; if set, enables memory/session DB
    #[arg(long)]
    session: Option<String>,

    /// Limit the number of items processed (for testing)
    #[arg(long)]
    limit: Option<usize>,

    /// Skip per-function ast-grep call-site analysis
    #[arg(long, action=ArgAction::SetTrue)]
    no_calls: bool,

    /// Skip per-function qualified path analysis
    #[arg(long, action=ArgAction::SetTrue)]
    no_paths: bool,

    /// Template for functions (expects response_format JSON)
    #[arg(long, default_value = "rustdoc_fn")]
    fn_template: String,

    /// Template for structs+fields (expects response_format JSON)
    #[arg(long, default_value = "rustdoc_struct")]
    struct_template: String,

    /// Awful Jade config file name under the app config dir
    /// (changed default to match the new init filename)
    #[arg(long, default_value = "rustdoc_config.yaml")]
    config: String,

    /// Only generate docs for these symbols (case-sensitive).
    #[arg(long = "only", value_delimiter = ',', value_name = "SYMBOL", num_args=1..)]
    only: Vec<String>,
}

// ------------------- Init helpers ----------------------------
/// Returns the path to the application's config directory.
///
/// Parameters:
/// - None
///
/// Returns:
/// - A `PathBuf` representing the directory path.
///
/// Errors:
/// - If `ProjectDirs::from` fails, an error is returned via `anyhow`.
///
/// Notes:
/// - The resulting path is OS-specific and follows standard XDG standards.
/// - On macOS, the directory is under `~/Library/Application Support/`.
/// - On Linux, it's under `~/.config/`.
/// - On Windows, it uses the %APPDATA% environment variable.
///
/// Examples:
/// ```no_run
/// let result = crate::config_root();
/// assert!(result.is_ok());
///
/// ```
fn config_root() -> Result<PathBuf> {
    // Resulting path like:
    // macOS: ~/Library/Application Support/com.awful-sec.awful_rustdocs
    // Linux: ~/.config/com.awful-sec/awful_rustdocs
    // Windows: %APPDATA%\com.awful-sec\awful_rustdocs
    let proj = ProjectDirs::from("com", "awful-sec", "aj")
        .ok_or_else(|| anyhow::anyhow!("could not determine OS config directory"))?;
    Ok(proj.config_dir().to_path_buf())
}

/// Write a file if it does not exist or is empty, unless `force` is true.
///
/// Parameters:
/// - `path`: The file path to check and write.
/// - `contents`: The string content to write into the file.
/// - `force`: Whether to overwrite an existing file (default: false).
///
/// Returns:
/// - `Ok(true)` if the file was written successfully, or
///   `Ok(false)` if the file already exists and `force` is false.
///
/// Errors:
/// - I/O errors when creating directories or writing the file,
///   or `path` is invalid.
///
/// Notes:
/// - If the directory of the file does not exist, it will be created.
///   If the file exists and `force` is false, this function returns immediately.
/// - The written data must be a valid UTF-8 string.
///
/// Examples:
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let path = std::path::Path::new("example.txt");
/// write_if_needed(path, "Hello, world!", false).unwrap();
/// # Ok(()) }
///
/// ```
fn write_if_needed(path: &std::path::Path, contents: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(true)
}

/// Initializes the configuration directory and writes default templates if needed.
///
/// Creates or updates configuration files in `config_root` if the `force` flag is set.
/// On dry run, prints what would be done without performing actions.
///
/// Parameters:
/// - `force`: If true, forces writing even if files already exist.
/// - `dry_run`: If true, simulates the operation without writing to disk.
///
/// Returns:
/// - `Ok(())` on success,
/// - Error if any I/O operations fail.
///
/// Errors:
/// - `ErrorKind::InvalidOperation` on dry run when output is required.
///
/// Safety:
/// - The function could mutate the filesystem in a way that could lead to data loss if --force is passed.
///
/// Notes:
/// - `write_if_needed` handles file writing logic, including checking for existing files.
///
/// Examples:
/// ```no_run
/// # #[cfg(not(dry-run))]
/// fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let config = config_root()?;
///     run_init(true, false)?;
/// # Ok(())
/// }
///
/// # #[cfg(dry-run)]
/// fn example_dry_run() -> Result<(), Box<dyn std::error::Error>> {
///     let config = config_root()?;
///     run_init(true, true)?;
/// # Ok(())
/// }
/// ```
fn run_init(force: bool, dry_run: bool) -> Result<()> {
    let root = config_root()?;
    let cfg = root.join("rustdoc_config.yaml");
    let tpl_dir = root.join("templates");
    let fn_tpl = tpl_dir.join("rustdoc_fn.yaml");
    let struct_tpl = tpl_dir.join("rustdoc_struct.yaml");

    if dry_run {
        eprintln!("Would create:");
        eprintln!("  {}", cfg.display());
        eprintln!("  {}", fn_tpl.display());
        eprintln!("  {}", struct_tpl.display());
        return Ok(());
    }

    let w1 = write_if_needed(&cfg, DEFAULT_CONFIG_YAML, force)?;
    let w2 = write_if_needed(&fn_tpl, DEFAULT_RUSTDOC_FN_YAML, force)?;
    let w3 = write_if_needed(&struct_tpl, DEFAULT_RUSTDOC_STRUCT_YAML, force)?;

    eprintln!("Config directory: {}", root.display());
    eprintln!("{} {}", if w1 { "Wrote" } else { "Kept" }, cfg.display());
    eprintln!("{} {}", if w2 { "Wrote" } else { "Kept" }, fn_tpl.display());
    eprintln!(
        "{} {}",
        if w3 { "Wrote" } else { "Kept" },
        struct_tpl.display()
    );
    Ok(())
}

/// A span of text with start and end positions in lines and bytes.
#[derive(Debug, Deserialize, Serialize, Clone)]
struct Span {
    /// Number of the line on which this span starts (1-based or 0-based, per OS).
    start_line: Option<u32>,
    /// Number of the line on which this span ends (1-based or 0-based, per OS)."
    end_line: Option<u32>,
    /// Offset of the start byte (as read from file, in zero-based bytes).
    start_byte: Option<u64>,
    /// Offset of the end byte (as read from file, in zero-based bytes).
    end_byte: Option<u64>,
}

/// A struct representing a code item (e.g. function, field) with metadata for processing.
#[derive(Debug, Deserialize, Serialize, Clone)]
struct Row {
    /// "fn", "struct", "field", etc.
    kind: String,
    /// name of a code item (tokenized, with special tokens)",
    name: String,
    /// "fn", "struct", "field", etc."}
    #[serde(rename = "crate")]
    crate_field: Option<String>,
    /// The normalized copy of crate_field
    crate_: Option<String>, // normalized copy of crate_field
    /// Path to the module (e.g. `crates/awesome-aj/srv`).
    module_path: Option<Vec<String>>,
    /// Fully qualified path (e.g. `crates/awesome-aj/srv::Row`)
    fqpath: String,
    /// How public this item is (e.g. "private", "public")
    visibility: String,
    /// The file that defines this item (e.g. `lib.rs`)
    file: String,
    /// Position in the source code
    span: Span,
    /// Signature without whitespace (e.g. "struct Row")
    signature: String,
    /// Whether this item has a body (e.g. true, false)
    has_body: bool,
    /// Documentation for this item
    doc: Option<String>,
    /// The body text (if any)
    body_text: Option<String>,
    // Added by your script:
    callers: Option<Vec<String>>,
}

/// Represents a documented item with metadata and content. Stores information about the type, location,
#[derive(Debug, Serialize)]
struct LlmDocResult {
    /// The type of documentation element (e.g., "fn", "struct")",
    kind: String,
    /// The fully-qualified path to the documented item (e.g., "crate::LlmDocResult")",
    fqpath: String,
    /// The source file where the documentation (or its replacement) was found",
    file: String,
    /// The starting line number of the documentation in the source file (0-based or 1-based?)
    start_line: Option<u32>,
    /// The ending line number of the documentation in the source file (inclusive)",
    end_line: Option<u32>,
    /// The function signature for this documentation element",
    signature: String,
    /// The list of callers that reference this documentation element",
    callers: Vec<String>,
    /// The symbols referenced by this documentation element",
    referenced_symbols: Vec<String>,
    /// The rendered documentation content as a multi-line string",
    llm_doc: String, // already rendered as /// lines
    /// Whether the documentation was already present before processing",
    had_existing_doc: bool,
}

// ------------------- ast-grep helpers (per-span) ----------------------------

/// A record representing a single file and its associated text range.
/// Contains metadata about the file, text content, and optional meta variables.
#[derive(Debug, Deserialize)]
struct SgRecord {
    /// The path to the file that this record references.
    file: String,
    /// The range of text in the file that this record refers to.
    /// This is serialized as `start`, `end`, and maybe a `type` value.
    /// it's used for precise positioning in the file.
    /// "The `range` field will be ignored for JSON" (see: https://github.com/jediboy/grammar-school/issues/63)
    /// "See [Range::with_json](crate::types::Range::with_json) for details."
    #[serde(rename = "range")]
    range: SgRange,
    /// Optional text content associated with the record.
    /// This is typically extracted from files, but can be empty or null if not present.
    text: Option<String>,
    /// Meta variables for the record, with default values.
    /// Defaults to an empty SgMetaVars instance if not provided.
    #[serde(default)]
    metaVariables: SgMetaVars,
}
/// Represents a range of bytes, serialized as `byteOffset`.
#[derive(Debug, Deserialize)]
struct SgRange {
    /// Specifies the byte offset of the range, serialized as `byteOffset`.
    #[serde(rename = "byteOffset")]
    byte: SgByteRange,
}
/// Represents a range of bytes in a file or buffer, defined by start and end offsets.
#[derive(Debug, Deserialize)]
struct SgByteRange {
    /// An unsigned 64-bit integer representing the starting byte offset in a file or buffer.
    start: u64,
    /// An unsigned 64-bit integer representing the ending byte offset in a file or buffer.
    end: u64,
}
/// The singleton meta variable container with all the information for a single key value.
/// It stores JSON values, which can be empty, strings or any other valid JSON type.
#[derive(Debug, Default, Deserialize)]
struct SgMetaVars {
    /// The associated value for a single key.
    #[serde(default)]
    single: serde_json::Value,
}

/// Represents a call site in code, with its kind and callee.
#[derive(Debug, Clone)]
struct CallSite {
    /// `kind` indicates the type of call site:
    kind: String, // "plain" | "qualified" | "method"
    /// `qual` is optional and holds an unsolved type qualifier.
    qual: Option<String>,
    /// `callee` is the name of a function, method, or type referenced by this.
    callee: String,
}

// ------------------- LLM struct JSON ---------------------------------------

/// Represents a field document output containing the name and its associated documentation string.
#[derive(Debug, Deserialize)]
struct FieldDocOut {
    /// The name of the field as defined in the reference text.
    name: String,
    /// The documentation string for the field, as extracted from the reference text.
    doc: String,
}

/// This struct stores documentation for `StructDocResponse`. It's used by `main` to provide human-readable descriptions of the fields and their values.
#[derive(Debug, Deserialize)]
struct StructDocResponse {
    /// The documentation text for the struct. It includes both the short summary above and the detailed explanation of what it represents.",
    struct_doc: String,
    /// A list of field documentation objects. Each contains metadata and a description of a struct or variant field.
    fields: Vec<FieldDocOut>,
}

// ------------------- Analysis helpers --------------------------------------
/// Handle the `calls_in_function_span` function to find call sites within a specific byte range of a file.
///
/// This function processes an AST (Abstract Syntax Tree) generated by `run_ast_grep_json` to identify call sites within a specified byte range. It supports three types of calls: plain, qualified, and method. Each call site is represented as a `CallSite` with information about the callee and optional qualifier.
///
/// Parameters:
/// - `file`: The path to the file being analyzed.
/// - `start_byte`: The starting byte in the file (inclusive) to search for call sites.
/// - `end_byte`: The ending byte in the file (inclusive) to search for call sites.
///
/// Returns:
/// - A `Result<Vec<CallSite>>` containing a list of call sites found within the specified byte range, or an error if any step fails.
///
/// Errors:
/// - I/O errors when reading the file or processing metadata.
/// - AST parsing errors from `run_ast_grep_json`.
///
/// Safety:
/// - The function is safe to use as long as the input file and metadata are valid.
///
/// Notes:
/// - This function uses patterns to identify call sites in the AST, and it handles different types of calls (plain, qualified, method) based on the pattern.
/// - The function is intended for use in tools that analyze call sites and memory usage during program execution.
///
/// Examples:
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let call_sites = calls_in_function_span("example_file.json", 0, 1024).await?;
/// for site in call_sites {
///     println!("Call site: {} {}", site.kind, site.callee);
/// }
/// # Ok(())
/// ```
fn calls_in_function_span(file: &str, start_byte: u64, end_byte: u64) -> Result<Vec<CallSite>> {
    let mut out = Vec::new();
    for (pat, kind) in [
        ("$N($$$A)", "plain"),
        ("$Q::$N($$$A)", "qualified"),
        ("$RECV.$N($$$A)", "method"),
    ] {
        let recs = run_ast_grep_json(file, pat)?;
        for r in recs {
            let s = r.range.byte.start;
            let e = r.range.byte.end;
            if s >= start_byte && e <= end_byte {
                let mv = &r.metaVariables.single;
                let name = mv
                    .pointer("/N/text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let qual = match kind {
                    "qualified" => mv
                        .pointer("/Q/text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    "method" => mv
                        .pointer("/RECV/text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    _ => None,
                };
                out.push(CallSite {
                    kind: kind.to_string(),
                    qual,
                    callee: name,
                });
            }
        }
    }
    Ok(out)
}

/// Handle a qualified path search within a file span.
///
/// This function searches for paths matching certain patterns in the AST of
/// `file` that lie between byte positions `start_byte` and `end_byte`.
/// It uses regular expressions to identify paths containing "::" in their text,
/// and inserts them into a sorted set of strings.
///
/// Parameters:
/// - `file`: The path to the file being analyzed.
/// - `start_byte`: The starting byte position in the file (inclusive).
/// - `end_byte`: The ending byte position in the file (inclusive).
///
/// Returns:
/// - A `Result` containing a sorted set of strings representing the qualified paths found.
///
/// Errors:
/// - I/O errors during file operations,
/// - JSON parsing/serialization errors from `run_ast_grep_json`,
/// - and any errors returned by the function body.
///
/// Safety:
/// - This is a private API and should not be used directly by end users.
///
/// Notes:
/// - The function is designed for internal use and may be unstable or change without notice.
///
/// Examples:
/// ```no_run
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let paths = qualified_paths_in_span("example.json", 10, 20)?;
/// assert!(paths.contains("some::path"));
/// # Ok(()) }
///
/// ```
fn qualified_paths_in_span(file: &str, start_byte: u64, end_byte: u64) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for pat in ["$Q::$N", "$Q::<$$$A>::$N", "$Q::{$$$A}"] {
        let recs = run_ast_grep_json(file, pat)?;
        for r in recs {
            let s = r.range.byte.start;
            let e = r.range.byte.end;
            if s >= start_byte && e <= end_byte {
                if let Some(txt) = r.text.as_ref() {
                    let t = txt.trim();
                    if t.contains("::") {
                        paths.insert(t.to_string());
                    }
                }
            }
        }
    }
    Ok(paths)
}

/// Run `ast-grep` to process a file and extract AST records.
///
/// Executes the `ast-grep` command with options to search for a pattern
/// in Rust files and return the resulting AST records as `SgRecord` objects.
///
/// # Parameters:
/// - `file`: The path to the Rust file or directory containing code.
/// - `pattern`: A regular expression pattern used to filter code lines.
///
/// # Returns:
/// A `Result` containing a vector of `SgRecord` objects if successful,
/// or an error indicating failure.
///
/// # Errors:
/// - Fails if `ast-grep` cannot be executed or exits with a non-zero status.
/// - Returns an error if the output is not valid UTF-8 or contains malformed JSON.
///
/// # Safety:
/// This function is marked as `private` and should not be called directly.
///
/// # Notes:
/// - The function uses the `serde_json` crate to parse output from `ast-grep`.
///   Ensure that the JSON format is correct, as invalid JSON will result in an error.
/// - The `ast-grep` command must be installed and available in the system's PATH.
///
/// # Examples:
/// ```rust
/// use anyhow::Result;
///
/// async fn example() -> Result<()> {
///     let file = "example.rs";
///     let pattern = r"#[derive]";
///     let recs = run_ast_grep_json(file, pattern).await?;
///
///     for rec in &recs {
///         println!("{:?}", rec);
///     }
///
///     Ok(())
/// }
///
/// ```
fn run_ast_grep_json(file: &str, pattern: &str) -> Result<Vec<SgRecord>> {
    let output = ProcCommand::new("ast-grep")
        .args([
            "run",
            "-l",
            "rust",
            "-p",
            pattern,
            "--json=stream",
            "--heading=never",
            "--color=never",
            file,
        ])
        .output()
        .context("running ast-grep")?;
    if !output.status.success() {
        anyhow::bail!("ast-grep failed for {}", file);
    }
    let mut recs = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<SgRecord>(line) {
            recs.push(rec);
        }
    }
    Ok(recs)
}

// ------------------- Main ---------------------------------------------------

/// Handle the main entry point for Awful Jade.
///
/// Initializes and processes configuration, loads templates,
/// harvests rows from Nushell scripts or files according to the CLI options,
/// indexes symbol names and groups items by file,
/// generates documentation for functions and structs using an LLM API,
/// handles serialization of results, and reports errors.
///
/// Parameters:
/// - `cli`: Command-line interface configuration parsed from input.
///
/// Returns:
/// - `Result<()>`, indicating success or failure due to I/O,
///   configuration loading, template parsing errors, etc.
///
/// Errors:
/// - I/O errors when reading/writing files,
///   YAML/JSON serialization failures,
///   and API call failures.
///
/// Notes:
/// - Only first `limit` items are processed unless --overwrite is used,
///   which allows reprocessing of already documented items.
/// - Struct documentation includes a single LLM call that returns both
///   the struct doc and per-field docs.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Command::Init { force, dry_run } => {
            run_init(force, dry_run)?;
            Ok(())
        }
        Command::Run(opts) => {
            // Load Awful Jade config (resolved by your loader under app config dir)
            let cfg_path: String = if Path::new(&opts.config).is_absolute() {
                opts.config.clone()
            } else {
                config_root()?
                    .join(&opts.config)
                    .to_string_lossy()
                    .into_owned()
            };

            // Now load using the absolute path
            let mut cfg: AwfulJadeConfig = load_config(&cfg_path).map_err(|e| {
                anyhow::anyhow!("Failed to load Awful Jade config: {cfg_path}: {e}")
            })?;

            if let Some(name) = &opts.session {
                cfg.ensure_conversation_and_config(name)
                    .await
                    .map_err(|e| anyhow::anyhow!("ensure_conversation_and_config failed: {}", e))?;
            }

            // Load templates
            let tpl_fn: ChatTemplate =
                template::load_template(&opts.fn_template)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to load template '{}': {}", opts.fn_template, e)
                    })?;
            let tpl_struct: ChatTemplate = template::load_template(&opts.struct_template)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to load struct template '{}': {}",
                        opts.struct_template,
                        e
                    )
                })?;

            // Targets
            let targets: Vec<String> = if opts.targets.is_empty() {
                vec![".".into()]
            } else {
                opts.targets
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect()
            };

            // Harvest all rows via Nushell
            let rows =
                run_nushell_harvest(&opts.script, &targets).context("nu + rust_ast.nu failed")?;

            // Index symbol names for heuristic reference matching
            let all_symbol_names: BTreeSet<String> = rows
                .iter()
                .map(|r| r.name.clone())
                .filter(|s| !s.is_empty())
                .collect();

            // Group items by file, honoring --only
            let mut per_file: BTreeMap<String, Vec<Row>> = BTreeMap::new();
            let want = |r: &Row| -> bool {
                if opts.only.is_empty() {
                    return true;
                }
                opts.only.iter().any(|s| s == &r.name || s == &r.fqpath)
            };
            for r in rows
                .iter()
                .filter(|r| (r.kind == "fn" || r.kind == "struct") && want(r))
            {
                per_file.entry(r.file.clone()).or_default().push(r.clone());
            }
            for v in per_file.values_mut() {
                v.sort_by_key(|r| (r.span.start_line.unwrap_or(0), r.fqpath.clone()));
            }

            if per_file.is_empty() && !opts.only.is_empty() {
                eprintln!("No items matched --only filter: {}", opts.only.join(", "));
            }

            // Build an index of functions by file for struct reference analysis
            let fn_rows: Vec<&Row> = rows.iter().filter(|r| r.kind == "fn").collect();

            let mut all_results: Vec<LlmDocResult> = Vec::new();
            let mut processed = 0usize;

            'files: for (_file, items) in per_file.iter() {
                for item in items {
                    if let Some(limit) = opts.limit {
                        if processed >= limit {
                            break 'files;
                        }
                    }
                    processed += 1;

                    let had_existing_doc = item
                        .doc
                        .as_ref()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false);
                    if had_existing_doc && !opts.overwrite {
                        if item.kind != "struct" {
                            continue;
                        }
                        // for structs we still proceed to allow field docs via single LLM call
                    }

                    match item.kind.as_str() {
                        "fn" => {
                            let mut referenced_symbols = referenced_symbols_in_body(
                                item.body_text.as_deref().unwrap_or(""),
                                &all_symbol_names,
                            );

                            let start_b = item.span.start_byte.unwrap_or(0);
                            let end_b = item.span.end_byte.unwrap_or(u64::MAX);

                            if !opts.no_paths {
                                let qpaths = qualified_paths_in_span(&item.file, start_b, end_b)
                                    .unwrap_or_default();
                                referenced_symbols.extend(qpaths.into_iter());
                            }

                            let calls_in_span = if opts.no_calls {
                                vec![]
                            } else {
                                calls_in_function_span(&item.file, start_b, end_b)
                                    .unwrap_or_default()
                            };

                            let question =
                                build_markdown_question(item, &referenced_symbols, &calls_in_span);

                            let answer = api::ask(&cfg, question, &tpl_fn, None, None)
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("LLM ask() failed for {}: {}", item.fqpath, e)
                                })?;

                            let llm_doc_block = sanitize_llm_doc(&answer);

                            all_results.push(LlmDocResult {
                                kind: "fn".into(),
                                fqpath: item.fqpath.clone(),
                                file: item.file.clone(),
                                start_line: item.span.start_line,
                                end_line: item.span.end_line,
                                signature: item.signature.clone(),
                                callers: item.callers.clone().unwrap_or_default(),
                                referenced_symbols,
                                llm_doc: llm_doc_block,
                                had_existing_doc,
                            });
                        }

                        "struct" => {
                            // 1) Find struct signature line and body block in the FILE SOURCE
                            let file_src = fs::read_to_string(&item.file)
                                .with_context(|| format!("reading {}", &item.file))?;

                            let struct_sig0 = match find_struct_sig_line_near(
                                &file_src,
                                item.span.start_line.unwrap_or(1).saturating_sub(1) as usize,
                            ) {
                                Some(l) => l,
                                None => {
                                    eprintln!(
                                        "warn: could not locate struct sig for {}",
                                        item.fqpath
                                    );
                                    continue;
                                }
                            };
                            let (body_lo, body_hi) =
                                match find_struct_body_block(&file_src, struct_sig0) {
                                    Some(p) => p,
                                    None => {
                                        eprintln!(
                                            "warn: could not locate struct body for {}",
                                            item.fqpath
                                        );
                                        continue;
                                    }
                                };
                            let body_text = extract_lines(&file_src, body_lo, body_hi);

                            // 2) Gather functions that reference this struct
                            let refs = referencing_functions(&item.name, &item.fqpath, &fn_rows);

                            // 3) Build struct prompt (expects JSON)
                            let question = build_struct_request_with_refs(item, &body_text, &refs);

                            let raw = api::ask(&cfg, question, &tpl_struct, None, None)
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("LLM ask() failed for {}: {}", item.fqpath, e)
                                })?;

                            // 4) Parse JSON; if it fails, degrade to plain struct doc only
                            let parsed: Result<StructDocResponse> = serde_json::from_str(&raw)
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "struct JSON parse failed for {}: {e}\nraw:\n{}",
                                        item.fqpath,
                                        raw
                                    )
                                });

                            let (struct_doc, field_docs): (String, Vec<FieldDocOut>) = match parsed
                            {
                                Ok(v) => (v.struct_doc, v.fields),
                                Err(err) => {
                                    eprintln!("{err}");
                                    (raw, vec![]) // fallback: assume whole payload is struct doc
                                }
                            };

                            // 5) Sanitize docs into `///` lines
                            let struct_llm_doc = sanitize_llm_doc(&struct_doc);

                            // 6) Map field names -> insertion points by scanning the actual struct body
                            let fields_in_file = extract_struct_fields_in_file(
                                &file_src,
                                body_lo,
                                body_hi,
                                &item.fqpath,
                            );
                            let mut field_index: BTreeMap<
                                String,
                                (usize /*insert_line0*/, String /*field_line_text*/),
                            > = BTreeMap::new();
                            for f in fields_in_file {
                                field_index.insert(f.name, (f.insert_line0, f.field_line_text));
                            }

                            // 7) Push struct doc result
                            all_results.push(LlmDocResult {
                                kind: "struct".into(),
                                fqpath: item.fqpath.clone(),
                                file: item.file.clone(),
                                start_line: item.span.start_line,
                                end_line: item.span.end_line,
                                signature: item.signature.clone(),
                                callers: item.callers.clone().unwrap_or_default(),
                                referenced_symbols: vec![],
                                llm_doc: struct_llm_doc,
                                had_existing_doc,
                            });

                            // 8) Push field doc results
                            for fd in field_docs {
                                if let Some((insert0, field_line_text)) =
                                    field_index.get(&fd.name).cloned()
                                {
                                    let doc_block = sanitize_llm_doc(&fd.doc);
                                    all_results.push(LlmDocResult {
                                        kind: "field".into(),
                                        fqpath: format!("{}::{}", item.fqpath, fd.name),
                                        file: item.file.clone(),
                                        start_line: Some((insert0 as u32) + 1),
                                        end_line: None,
                                        signature: field_line_text,
                                        callers: vec![],
                                        referenced_symbols: vec![],
                                        llm_doc: doc_block,
                                        had_existing_doc: false,
                                    });
                                } else {
                                    eprintln!(
                                        "warn: field '{}' not found in {} — skipping doc",
                                        fd.name, item.fqpath
                                    );
                                }
                            }
                        }

                        _ => {}
                    }
                }
            }

            // Persist results
            let out_dir = Utf8PathBuf::from("target/llm_rustdocs");
            fs::create_dir_all(&out_dir)?;
            let out_json = out_dir.join("docs.json");
            fs::write(&out_json, serde_json::to_vec_pretty(&all_results)?)?;
            eprintln!("Wrote {}", out_json);

            // Patch source files
            if opts.write {
                patch_files_with_docs(&all_results, opts.overwrite)?;
            }

            Ok(())
        }
    }
}

// ------------------- Prompt builders ---------------------------------------

/// Handle the `build_markdown_question` function, which generates a structured Rustdoc comment block for a given function.
///
/// This utility constructs a comprehensive documentation string by extracting and formatting metadata about the provided `Row`.
/// It includes details such as function identity, existing documentation (if any), referenced symbols, and internal calls.
///
/// # Parameters
/// - `f`: A reference to a `Row` struct containing metadata about the function.
/// - `referenced_symbols`: A slice of string references representing symbols referenced within the function.
/// - `calls_in_span`: A slice of `CallSite` structs representing internal function calls made within the scope.
///
/// # Returns
/// A `String` containing a formatted Rustdoc comment block, suitable for inclusion in source code.
///
/// # Notes
/// - The function extracts and formats metadata to generate a clear, structured documentation string.
/// - If existing Rustdoc is present, it is extracted and reformatted without modification.
/// - The function supports truncating lengthy function bodies for clarity.
fn build_markdown_question(
    f: &Row,
    referenced_symbols: &[String],
    calls_in_span: &[CallSite],
) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    writeln!(s, "# Rust Function Documentation Task").ok();
    writeln!(s, "You are given context about a single Rust function.").ok();
    writeln!(s).ok();

    writeln!(s, "## Function Identity").ok();
    writeln!(s, "- **Fully-qualified path**: `{}`", f.fqpath).ok();
    writeln!(s, "- **Signature**: `{}`", f.signature).ok();
    writeln!(s, "- **Visibility**: `{}`", f.visibility).ok();

    writeln!(s, "\n## Existing Documentation").ok();
    match &f.doc {
        Some(doc) if !doc.trim().is_empty() => {
            writeln!(
                s,
                "The function already has Rustdoc. Improve and rewrite it if necessary:"
            )
            .ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => {
            writeln!(s, "_No existing rustdoc found._").ok();
        }
    };

    writeln!(s, "\n## Referenced Symbols (body-level)").ok();
    if referenced_symbols.is_empty() {
        writeln!(s, "_No symbol references detected._").ok();
    } else {
        for sym in referenced_symbols {
            writeln!(s, "- `{}`", sym).ok();
        }
    }

    if !calls_in_span.is_empty() {
        writeln!(s, "\n## Function Calls Inside This Function").ok();
        for c in calls_in_span.iter().take(50) {
            match &c.qual {
                Some(q) => {
                    writeln!(s, "- **{}** call → `{}` on `{}`", c.kind, c.callee, q).ok();
                }
                None => {
                    writeln!(s, "- **{}** call → `{}`", c.kind, c.callee).ok();
                }
            };
        }
    }

    if let Some(body) = &f.body_text {
        writeln!(s, "\n## Function Body (Truncated)").ok();
        let trimmed = truncate_for_context(body, 8000, 400);
        writeln!(s, "```rust\n{}\n```", trimmed).ok();
    }

    writeln!(
        s,
        "\n---\n## Output Requirements\n\
         Return **ONLY** a Rustdoc block composed of lines starting with `///`.\n\
         - No JSON, no backticks, no XML, no surrounding prose.\n\
         - Include a clear 1–2 sentence summary.\n\
         - If relevant, add sections titled exactly: `Parameters:`, `Returns:`, `Errors:`, `Safety:`, `Notes:`, `Examples:`.\n\
         - Use concise bullet points; examples should be doc-test friendly (no fenced code).\n\
         - Every line MUST start with `///` (or be a blank `///`)."
    ).ok();

    s
}

/// Handles generating a structured Rustdoc for a given struct, including its identity, existing documentation, body text, and referencing functions.
///
/// Used to create a detailed documentation template for structs in Rust projects, especially when generating or updating docs automatically.
///
/// Parameters:
/// - `srow`: A reference to a struct metadata object containing its fully-qualified path, signature, visibility, and existing documentation.
/// - `body_text`: The raw source code of the struct's body (as a string).
/// - `referencing_fns`: A slice of function names that reference this struct.
///
/// Returns:
/// - A formatted string containing the full Rustdoc for the struct, including its summary, documentation sections, and output requirements.
///
/// Errors:
/// - None explicitly documented; function is designed to generate documentation rather than handle errors.
///
/// Safety:
/// - This function does not perform unsafe operations; it is used for documentation generation only.
///
/// Notes:
/// - The output format requires structured JSON with specific keys like `struct_doc` and `fields`.
/// - Example: A simple struct with one field, including a brief summary in its documentation.
fn build_struct_request_with_refs(
    srow: &Row,
    body_text: &str,
    referencing_fns: &[String],
) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    writeln!(s, "# Rust Struct Documentation Task").ok();
    writeln!(s, "You are given the source of a single Rust struct and a list of functions that reference it.").ok();

    writeln!(s, "\n## Struct Identity").ok();
    writeln!(s, "- **Fully-qualified path**: `{}`", srow.fqpath).ok();
    writeln!(s, "- **Signature**: `{}`", srow.signature).ok();
    writeln!(s, "- **Visibility**: `{}`", srow.visibility).ok();

    writeln!(s, "\n## Existing Documentation").ok();
    match &srow.doc {
        Some(doc) if !doc.trim().is_empty() => {
            writeln!(
                s,
                "The struct already has Rustdoc. If needed, rewrite it to be concise:"
            )
            .ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => {
            writeln!(s, "_No existing rustdoc found._").ok();
        }
    };

    writeln!(s, "\n## Struct Body (verbatim)").ok();
    writeln!(s, "```rust\n{}\n```", body_text).ok();

    writeln!(s, "\n## Referencing Functions (FQ paths)").ok();
    if referencing_fns.is_empty() {
        writeln!(s, "_No referencing functions detected in the crate._").ok();
    } else {
        for f in referencing_fns.iter().take(100) {
            writeln!(s, "- `{}`", f).ok();
        }
    }

    writeln!(s, "\n---\n## Output Requirements").ok();
    writeln!(
        s,
        "Respond in **structured JSON** (no prose) with this shape:"
    )
    .ok();
    writeln!(
        s,
        r#"{{
  "struct_doc": "/// short summary...\n/// ...",
  "fields": [
    {{ "name": "field_name", "doc": "/// one-line or short doc...\n/// ..." }}
  ]
}}"#
    )
    .ok();
    writeln!(
        s,
        "- `struct_doc`: A short 1–2 sentence rustdoc for the struct (above attributes)."
    )
    .ok();
    writeln!(s, "- `fields`: One entry **per named field** appearing in the struct body; the `doc` value must be a ready-to-insert `///` block for that field (keep it short, include units/invariants if relevant).").ok();

    s
}

// ------------------- File parsing for structs & fields ----------------------

/// A struct representing field specifications for memory items. It stores metadata about fields in a structured format.
#[derive(Debug)]
struct FieldSpec {
    /// The name of the field.
    name: String,
    /// The starting line index (0-based) of the field in a file.
    field_line0: usize,
    /// The line index (0-based) where the field should be inserted.
    insert_line0: usize,
    /// The fully qualified path of the parent struct, if any.
    parent_fqpath: String,
    /// The content of the field as a string.
    field_line_text: String,
}

/// Extracts a range of lines from a string, filtering by line indices.
///
/// Parameters:
/// - `src`: The input string to extract lines from.
/// - `lo_line0`: The starting line index (inclusive).
/// - `hi_line0`: The ending line index (inclusive).
///
/// Returns:
/// A string containing the extracted lines joined by newlines.
///
/// Examples:
/// ```rust
/// let input = "Line 1
/// Line 2
/// Line 3";
/// assert_eq!(
///     extract_lines(input, 0, 1),
///     "Line 1
/// Line 2"
/// );
/// ```
///
/// Notes:
/// - Line indices are zero-based and correspond to the lines returned by `str::lines()`.
/// - This function is safe for any valid input.
fn extract_lines(src: &str, lo_line0: usize, hi_line0: usize) -> String {
    src.lines()
        .enumerate()
        .filter(|(i, _)| *i >= lo_line0 && *i <= hi_line0)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Finds a line near the given line number where a `struct` declaration is likely to be found.
///
/// This function searches for the closest line containing a `struct` declaration
/// by scanning forward and backward from the provided start line. It uses a regular expression to match `struct` keywords.
///
/// Parameters:
/// - `src`: A string slice representing the source code to search through.
/// - `start_line0`: The line number from which to begin searching (0-based).
///
/// Returns:
/// - `Some(usize)`: The line number where a `struct` declaration was found, or
///   - `None` if no such line exists within the specified range.
///
/// Notes:
/// - The function searches within 20 lines forward and 5 lines backward from the start line.
/// - If a `struct` is found beyond that range, it will not be returned as no further checks are made.
///
/// Examples:
/// ```rust
/// let src = "pub struct Example {}; pub fn main() { }";
/// let result = find_struct_sig_line_near(src, 0);
/// assert_eq!(result, Some(0));
///
/// let src = "pub fn main() { }
/// pub struct Example {};";
/// let result = find_struct_sig_line_near(src, 1);
/// assert_eq!(result, Some(1));
///
/// ```
fn find_struct_sig_line_near(src: &str, start_line0: usize) -> Option<usize> {
    let re_struct = Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\b"#).unwrap();
    let total = src.lines().count();
    for i in start_line0.min(total)..(start_line0 + 20).min(total) {
        if src
            .lines()
            .nth(i)
            .map(|l| re_struct.is_match(l))
            .unwrap_or(false)
        {
            return Some(i);
        }
    }
    let up_lo = start_line0.saturating_sub(5);
    for i in (up_lo..start_line0.min(total)).rev() {
        if src
            .lines()
            .nth(i)
            .map(|l| re_struct.is_match(l))
            .unwrap_or(false)
        {
            return Some(i);
        }
    }
    None
}

/// Finds the body of a struct in a given source string, starting from a specified line.
///
/// Parameters:
/// - `src`: The input string to search within.
/// - `struct_sig_line0`: The line index where the struct signature begins (inclusive).
///
/// Returns:
/// - `Some((start, end))` if a struct body is found, where `start` is the line number of the opening brace and `end` is the line number of the closing brace.
/// - `None` if no struct body is found or an error occurs during parsing.
///
/// Notes:
/// - This function assumes the input string is properly formatted and contains a struct definition.
/// - It counts opening and closing braces to determine the start and end of the struct body.
///
/// Examples:
/// ```rust
/// assert_eq!(find_struct_body_block("struct Example { ... }", 0), Some((0, 1)));
/// assert_eq!(find_struct_body_block("struct Example { ... }", 2), None);
///
/// ```
fn find_struct_body_block(src: &str, struct_sig_line0: usize) -> Option<(usize, usize)> {
    let mut brace_line_start = None;
    let mut open = 0i32;
    for (i, line) in src.lines().enumerate().skip(struct_sig_line0) {
        if brace_line_start.is_none() {
            if let Some(pos) = line.find('{') {
                brace_line_start = Some((i, pos));
                open = 1;
            }
            continue;
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    open += 1;
                }
                if ch == '}' {
                    open -= 1;
                }
            }
            if open == 0 {
                let (start, _) = brace_line_start.unwrap();
                return Some((start, i));
            }
        }
    }
    None
}

/// Extracts field specifications from a file's struct body.
///
/// Parses lines within a specified range to identify fields in a struct,
/// capturing their names, positions, and context for use in field extraction logic.
///
/// Parameters:
/// - `file_src`: The source string containing the file's content to parse.
/// - `body_start_line0`: Line number marking the start of the struct body (inclusive).
/// - `body_end_line0`: Line number marking the end of the struct body (inclusive).
/// - `parent_fqpath`: The fully qualified path to the parent module, used for field reference resolution.
///
/// Returns:
/// - `Vec<FieldSpec>`: A list of fields identified, each containing metadata about its position and name.
///
/// Errors:
/// - Bubbles up errors from regex compilation (`Regex::new`) and file parsing.
///
/// Notes:
/// - This function is used internally by `extract_struct_fields_between()` and should not be called directly.
/// - It assumes the input file contains valid Rust struct syntax within a specified line range.
///
/// Examples:
/// ```rust
/// let file_content = r#"
/// struct Example {
///     pub field1: i32,
///     field2: String,
/// };
/// "#;
///
/// let struct_fields = extract_struct_fields_in_file(
///     file_content,
///     4, // start of struct body (line 0 is the opening '{')
///     7, // end of struct body (line 7 is the closing '}')
///     "crate::example::mod",
/// );
///
/// assert_eq!(struct_fields.len(), 2);
///
/// ```
fn extract_struct_fields_in_file(
    file_src: &str,
    body_start_line0: usize,
    body_end_line0: usize,
    parent_fqpath: &str,
) -> Vec<FieldSpec> {
    let lines: Vec<&str> = file_src.lines().collect();
    let mut out = Vec::new();

    let attr_re = Regex::new(r#"^\s*#\["#).unwrap();
    let field_re = Regex::new(
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*[^;{]+?,?\s*(?://.*)?$"#,
    )
    .unwrap();

    let mut i = body_start_line0 + 1; // after the '{'
    while i < lines.len() && i <= body_end_line0.saturating_sub(1) {
        let mut j = i;
        let mut attr_top = j;
        while j <= body_end_line0 && j < lines.len() && attr_re.is_match(lines[j].trim_start()) {
            j += 1;
        }
        if j <= body_end_line0 && j < lines.len() {
            let l = lines[j];
            if field_re.is_match(l) {
                let caps = field_re.captures(l).unwrap();
                let name = caps
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    let insert_line0 = if attr_top < j { attr_top } else { j };
                    out.push(FieldSpec {
                        name,
                        field_line0: j,
                        insert_line0,
                        parent_fqpath: parent_fqpath.to_string(),
                        field_line_text: l.to_string(),
                    });
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }

    out
}

// ------------------- Referencing functions for a struct ---------------------

/// Handle function references by matching a struct name or fully qualified path in documentation text.
///
/// Parameters:
/// - `struct_name`: The name of the struct to search for.
/// - `struct_fq`: The fully qualified path (including namespace) of the struct to search for.
/// - `fns`: A slice of documentation rows containing text and paths to check.
///
///
/// Returns:
/// - `Vec<String>`: A collection of fully qualified paths that match either the struct name or path.
///
///
/// Safety:
/// - This function is safe for use in general Rust code, but uses regular expressions which may have performance implications.
///
///
/// Notes:
/// - The function searches for occurrences of the struct name or fully qualified path within documentation text.
/// - It uses regex to match patterns and is intended for use in analyzing or querying documentation data.
///
///
/// Examples:
/// ```no_run
/// let fns = vec![Row { body_text: "This is a function for MyStruct::MySub".to_string() }];
/// let result = referencing_functions("MyStruct", "MyStruct::MySub", &fns);
/// assert_eq!(result, vec!["MyStruct::MySub".to_string()]);
///
/// ```
fn referencing_functions(struct_name: &str, struct_fq: &str, fns: &[&Row]) -> Vec<String> {
    let word_name = Regex::new(&format!(r"\b{}\b", regex::escape(struct_name))).unwrap();
    let word_fq = Regex::new(&regex::escape(struct_fq)).unwrap(); // fq may include ::; treat as literal

    let mut out = Vec::new();
    for f in fns {
        let body = f.body_text.as_deref().unwrap_or("");
        if word_name.is_match(body) || word_fq.is_match(body) {
            out.push(f.fqpath.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

// ------------------- Nushell harvest ---------------------------------------

/// Executes a Nu script to harvest JSON data from Rust-AST.
///
/// This function creates a temporary file containing a Nu script that sources
/// the provided Rust AST and exports its contents to JSON. It then executes
/// this script through `nu`, captures the output, and parses it into a vector of
/// JSON rows. Optionally fields are added to store the crate name and file path.
///
/// Parameters:
/// - `script_path`: Path to the Rust AST source file.
/// - `targets`: Optional list of targets (file paths) for harvesting.
///
/// Returns:
/// - A `Result` containing the parsed JSON rows on success,
///   or an error with context otherwise.
///
/// Errors:
/// - I/O errors when creating/temp files, reading/writing stdout,
///   or parsing JSON.
///
/// Safety:
/// - Use with caution, as it executes external scripts and may
///   expose sensitive data.
///
/// Notes:
/// - The script uses Nu commands to process the AST and export results.
///   The default target is `.` (current directory).
/// - If no targets are provided, it harvests all results.
///
/// Examples:
/// ```no_run
/// # async fn example() -> Result<Vec<Row>> {
/// let rows = run_nushell_harvest("/path/to/file.rs", &["src/main.rs"]).await?;
/// # Ok(rows) }
///
/// ```
fn run_nushell_harvest(script_path: &Utf8PathBuf, targets: &[String]) -> Result<Vec<Row>> {
    // tiny wrapper Nu script
    let mut tmp = NamedTempFile::new()?;
    let mut content = String::new();
    content.push_str(&format!("source {}\n", shell_escape(script_path.as_str())));
    content.push_str("let rows = (rust-ast ");
    if targets.is_empty() {
        content.push_str("."); // default
    } else {
        for t in targets {
            content.push_str(&format!(" {}", shell_escape(t)));
        }
    }
    content.push_str(")\n($rows | to json)\n");
    tmp.write_all(content.as_bytes())?;

    let mut cmd = ProcCommand::new("nu");
    cmd.arg("--no-config-file")
        .arg(tmp.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().context("spawning nu failed")?;
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .context("reading nu stdout failed")?;
    let status = child.wait().context("waiting for nu failed")?;
    if !status.success() {
        anyhow::bail!("nu exited with non-zero status");
    }

    let mut rows: Vec<Row> =
        serde_json::from_str(&stdout).context("parsing rust-ast JSON failed")?;
    for r in &mut rows {
        if r.crate_.is_none() && r.crate_field.is_some() {
            r.crate_ = r.crate_field.clone();
        }
    }
    Ok(rows)
}

// ------------------- Utilities ---------------------------------------------

/// Truncates a string to fit within specified character and line limits.
/// This function takes an input string, trims it to the maximum number of lines specified by `max_lines`,
/// and further truncates it if its length exceeds the maximum character limit, appending a truncation marker.
///
/// Parameters:
/// - `s`: The input string to be truncated.
/// - `max_chars`: The maximum number of characters allowed in the output string.
/// - `max_lines`: The maximum number of lines to retain from the input.
///
/// Returns:
/// - A truncated version of the input string, with a marker indicating truncation if necessary.
///
/// Errors:
/// - This function does not return any explicit errors; it handles truncation internally.
///
/// Notes:
/// - If the truncated length exceeds `max_chars`, a marker (`// …truncated…`) is appended.
/// - The function uses line-by-line processing and collects results into a `Vec<String>`.
///
/// Examples:
/// ```rust
/// assert_eq!(truncate_for_context("This is a long string that needs to be truncated.", 10, 2),
///            "This is a ...
/// // …truncated…");
///
/// ```
fn truncate_for_context(s: &str, max_chars: usize, max_lines: usize) -> String {
    let mut out = s.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    if out.len() > max_chars {
        out.truncate(max_chars);
        out.push_str("\n// …truncated…");
    }
    out
}

/// Handle a body of text and extract referenced symbols from `all_symbols`.
///
/// This function uses a regex to find all identifiers in the input body text and
/// filters them against `all_symbols`. It returns up to 64 unique matches.
///
/// Parameters:
/// - `body`: A string slice containing the text to analyze.
/// - `all_symbols`: A reference to a BTreeSet of strings representing symbols
///   known in the context.
///
/// Returns:
/// - A Vec<String> containing up to 64 unique identifiers found in the body.
///
/// Errors:
/// - None are returned directly; all errors are bubbled up from underlying methods.
///
/// Notes:
/// - The regex matches identifiers starting with a letter or underscore, followed
///   by letters, digits, or underscores.
/// - The `BTreeSet` guarantees ordered iteration for stability.
///
/// Examples:
/// ```rust
/// let body = "fn hello() { let x: i32 = 42; }";
/// let all_symbols = BTreeSet::from(["hello", "x"]);
/// let result = referenced_symbols_in_body(body, &all_symbols);
/// assert_eq!(result, vec!["x".to_string()]);
///
/// ```
fn referenced_symbols_in_body(body: &str, all_symbols: &BTreeSet<String>) -> Vec<String> {
    if body.is_empty() {
        return vec![];
    }
    let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();
    let found: BTreeSet<String> = re
        .find_iter(body)
        .map(|m| m.as_str().to_string())
        .filter(|w| all_symbols.contains(w))
        .collect();
    found.into_iter().take(64).collect()
}

/// Escapes a string for use in shell commands by surrounding non-alphanumeric and non-"/._-" characters with single quotes.
///
/// Parameters:
/// - `s`: The string to be escaped.
///
/// Returns:
/// - A new `String` with the input text properly escaped for shell use.
///
/// Errors:
/// - None
///
/// Notes:
/// - The function checks each character of the input string. If all characters are alphanumeric or one of `/._-`, it returns the original string.
/// - Otherwise, it escapes all characters by surrounding them with single quotes and escaping any existing apostrophes.
///
/// Examples:
/// ```rust
/// assert_eq!(shell_escape("hello"), "hello");
/// assert_eq!(shell_escape("hello/world!"), "'hello/world!'");
///
/// ```
fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

// ------------------- Patcher ------------------------------------------------
//
/// Patches documentation into Rust source files based on `LlmDocResult` items.
///
/// This function processes a list of documentation results (`LlmDocResult`) and applies them to
/// source files, inserting or updating doc comments in a way that respects item signatures,
/// struct attributes, and field definitions. It supports overwriting existing docs when
/// appropriate.
///
/// Parameters:
/// - `results`: A slice of documentation items to apply, each with a file path and line
///   information.
/// - `overwrite`: Whether to overwrite existing documentation blocks when they are found.
///
/// Returns:
/// - A `Result<()>`, indicating success or failure in patching the files.
///
/// Errors:
/// - I/O errors when reading/writing files,
/// - Regex/line parsing failures in identifying item signatures and doc insertion ranges.
///
/// Safety:
/// - This function is meant to be called from trusted environments where file contents are
///   controlled. It does not validate the integrity of source files.
///
/// Notes:
/// - Documentation is inserted **directly above** struct attributes or field definitions,
///   ensuring clarity and proper formatting.
/// - If a file has no existing documentation, it will be inserted at the appropriate line.
/// - The function uses regex to identify item signatures and doc blocks, which may have
///   false positives in complex codebases.
///
/// Examples:
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let results = vec![...]; // A slice of LlmDocResult items
/// patch_files_with_docs(&results, true).await?;
/// # Ok(()) }
///
/// ```
fn patch_files_with_docs(results: &[LlmDocResult], overwrite: bool) -> Result<()> {
    use regex::Regex;

    // Heuristics for item "signature" lines
    let re_fn = Regex::new(
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\b"#
    ).unwrap();
    let re_struct = Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\b"#).unwrap();
    // Field line inside a struct: `name: Type,` (allow pub(...) and trailing comma)
    let re_field =
        Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?[A-Za-z_][A-Za-z0-9_]*\s*:\s*[^;{}]+,?\s*$"#)
            .unwrap();

    /// Checks whether a given line in source code is likely to be an item signature using regex matching.
    ///
    /// Parameters:
    /// - `src`: The source code string to search within.
    /// - `line_idx0`: The index of the line in `src` to check for an item signature.
    /// - `re`: A regex pattern used to determine if the line matches an item signature.
    ///
    /// Returns:
    /// - `true` if the specified line in `src` matches the regex pattern, otherwise `false`.
    ///
    /// Notes:
    /// - If the specified line is out of bounds, it returns `false`.
    /// - The regex pattern must match exactly the entire line.
    ///
    /// Examples:
    /// ```rust
    /// assert!(is_probably_item_sig("class Item;
    /// // item signature", 0, r#"/^class\s+Item;/m"#));
    /// assert!(!is_probably_item_sig("function foo()", 0, r#"r#"/^class\s+Item;/m"#));
    ///
    /// ```
    #[inline]
    fn is_probably_item_sig(src: &str, line_idx0: usize, re: &Regex) -> bool {
        src.lines()
            .nth(line_idx0)
            .map(|line| re.is_match(line))
            .unwrap_or(false)
    }

    // Finds the most likely signature line near an approximate starting line.
    /// Find a line near the starting point that matches a regular expression.
    ///
    /// Scans lines in `src` up to 20 lines below the starting point, then a small
    /// backward scan (up to 5 lines above) to handle edge cases like off-by-one errors.
    /// Returns the first matching line index or `None` if no match is found.
    ///
    /// Parameters:
    /// - `src`: The input string to search through.
    /// - `start_line0`: The initial line number to start scanning from.
    /// - `re`: A regular expression used to match lines containing item signatures.
    ///
    /// Returns:
    /// - `Some<usize>`: The index of the matching line, or
    /// - `None` if no match is found.
    ///
    /// Notes:
    /// - Scans are limited to the bounds of `src`'s lines.
    /// - The small upward scan ensures proper handling of edge cases like body-start anchors.
    ///
    /// Examples:
    /// ```no_run
    /// # fn example() -> Option<usize> {
    /// let src = "line1
    /// line2
    /// line3";
    /// let re = regex::Regex::new(r"\\d").unwrap();
    /// find_item_sig_line_near(src, 1, &re)
    /// # }
    ///
    /// // Expected to return Some(0) if the regex matches "line1".
    ///
    /// ```
    fn find_item_sig_line_near(src: &str, start_line0: usize, re: &Regex) -> Option<usize> {
        let total = src.lines().count();
        // look down up to 20 lines
        for i in start_line0.min(total)..(start_line0 + 20).min(total) {
            if is_probably_item_sig(src, i, re) {
                return Some(i);
            }
        }
        // small look-up (handles off-by-one and body-start anchors)
        let up_lo = start_line0.saturating_sub(5);
        for i in (up_lo..start_line0.min(total)).rev() {
            if is_probably_item_sig(src, i, re) {
                return Some(i);
            }
        }
        None
    }

    // Returns (lo, hi) in **line indices** such that the replacement byte range is
    // [ byte(lo), byte(hi) ), i.e., it ends at the start of `hi` (exclusive).
    // Places struct docs **directly above the first attribute line** (no blank line).
    // If there is already a contiguous `///` block immediately above attributes:
    //   - returns that block range when `overwrite = true`
    //   - returns None when `overwrite = false`
    /// Handle insertion of documentation above a struct's attribute block.
    ///
    /// This function searches upward from the `struct_sig_line0` to find the first attribute block
    /// immediately above it. It optionally inserts a contiguous block of `///` documentation
    /// just before the first attribute (or directly above the struct if no attributes are found).
    ///
    /// # Parameters:
    /// - `src`: The source string containing the code.
    /// - `struct_sig_line0`: The line number of the struct signature in question.
    /// - `overwrite`: Whether to overwrite existing documentation blocks instead of inserting new ones.
    ///
    /// # Returns:
    /// - `Some((usize, usize))`: The range of lines where documentation was inserted.
    ///   - First index (`doc_lo`) is the start line.
    ///   - Second index (`anchor`) is the end line (typically one less than `doc_lo`).
    /// - `None`: If no insertion was performed (e.g., due to an existing block and `overwrite = false`).
    ///
    /// # Notes:
    /// - If blank lines exist between attributes and the signature, they are skipped.
    /// - The function identifies `#[` or `#![` as attribute markers and stops searching above them.
    /// - If an existing documentation block is found, it may be replaced or ignored depending on `overwrite`.
    ///
    /// # Examples:
    /// ```no_run
    /// let src = "#[derive(Debug)]
    /// struct Example;";
    /// let range = struct_doc_insertion_range_above_attrs(src, 2, true).unwrap();
    /// assert_eq!(range, (0, 1));
    ///
    /// ```
    fn struct_doc_insertion_range_above_attrs(
        src: &str,
        struct_sig_line0: usize,
        overwrite: bool,
    ) -> Option<(usize, usize)> {
        let lines: Vec<&str> = src.lines().collect();

        // Walk upward to find the *first* attribute in the attribute block just above the struct.
        // Also absorb a single blank line that sometimes separates attributes from the signature.
        let mut attr_first = struct_sig_line0; // default to the signature if no attributes
        let mut i = struct_sig_line0.saturating_sub(1);
        let mut saw_attr = false;
        while i < lines.len() {
            if i == usize::MAX {
                break;
            }
            let t = lines[i].trim_start();
            if t.starts_with("#[") || t.starts_with("#![") {
                saw_attr = true;
                attr_first = i;
                if i == 0 {
                    break;
                }
                i = i.saturating_sub(1);
                continue;
            }
            // If we see a blank *immediately* above attributes, remember the attr_first;
            // but we will not keep the blank between doc and attributes (we'll remove it).
            if t.is_empty() && saw_attr {
                // keep scanning upward once to see if there are more attributes (rare),
                // but do not move attr_first past the blank.
                if i == 0 {
                    break;
                }
                i = i.saturating_sub(1);
                continue;
            }
            break;
        }

        // We want docs inserted **just above** the first attribute (or the struct line if no attributes).
        let anchor = attr_first;

        // Is there already a contiguous block of `///` immediately above `anchor`?
        if anchor > 0 && lines[anchor - 1].trim_start().starts_with("///") {
            let mut doc_lo = anchor - 1;
            while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") {
                doc_lo -= 1;
            }
            if !overwrite {
                return None;
            }
            // Replace exactly the existing doc block; keep attributes untouched.
            return Some((doc_lo, anchor));
        }

        // No existing docs → insert at anchor (lo == hi), so the doc sits directly above attributes.
        Some((anchor, anchor))
    }

    // Field doc insertion: insert **immediately above** the given insertion anchor `insert_line0`
    // (which should be the field line index), or replace the existing contiguous block of `///`
    // if it already exists there and `overwrite = true`.
    /// Handle a field doc insertion by finding the appropriate location in a source string for an `///`-formatted doc block.
    ///
    /// This function searches through the lines of a string to find the first occurrence of `///` before the specified insert position.
    /// If found, it returns the range between this doc block and the insertion point for field documentation.
    ///
    /// Parameters:
    /// - `src`: The source string to search within.
    /// - `insert_line0`: The line index where the field should be inserted.
    /// - `overwrite`: Whether to overwrite an existing doc block if one is found.
    ///
    /// Returns:
    /// - `Option<(usize, usize)>`: A tuple of line indices defining the range where documentation should be inserted.
    ///   - `None` if no suitable location is found or the overwrite flag is false and an existing doc block exists.
    ///
    /// Notes:
    /// - This function handles UTF-8 correctly as per `str::lines()`.
    /// - It supports doc blocks that may span multiple lines by checking for leading `///` on any line.
    ///
    /// Examples:
    /// ```rust
    /// assert_eq!(field_doc_insertion_range("///Example
    /// ", 2, true), Some((0, 2)));
    /// assert_eq!(field_doc_insertion_range("///Another
    /// Example", 4, true), Some((0, 2)));
    /// assert_eq!(field_doc_insertion_range("Example", 2, false), Some((0, 2)));
    ///
    /// ```
    fn field_doc_insertion_range(
        src: &str,
        insert_line0: usize,
        overwrite: bool,
    ) -> Option<(usize, usize)> {
        let lines: Vec<&str> = src.lines().collect();
        if insert_line0 == 0 {
            return Some((0, 0));
        }
        let i = insert_line0 - 1;
        if lines
            .get(i)
            .map_or(false, |l| l.trim_start().starts_with("///"))
        {
            // there's already a doc block directly above the field
            if !overwrite {
                return None;
            }
            let mut doc_lo = i;
            while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") {
                doc_lo -= 1;
            }
            return Some((doc_lo, insert_line0));
        }
        // No existing docs → insert at insert_line0
        Some((insert_line0, insert_line0))
    }

    /// Returns a regex based on the kind provided.
    /// If `kind` is "struct", returns `re_struct`.
    /// If `kind` is "field", returns `re_field`..
    /// Otherwise, returns `re_fn`.
    ///
    /// Parameters:
    /// - `kind`: A string indicating which regex to return ("struct", "field", or default).
    /// - `re_fn`: A generic regex.
    /// - `re_struct`: A regex for struct fields.
    /// - `re_field`: A regex for field names.
    ///
    /// Returns:
    /// A reference to a regex based on the `kind` parameter.
    ///
    /// Notes:
    /// The function uses a match statement to determine which regex to return based on the `kind` parameter.
    /// The returned value is a reference, so it does not own the data but points to an existing one.
    ///
    /// Examples:
    /// ```rust
    /// assert_eq!(re_for_kind("struct", &regex::Regex::new(r".*").unwrap(), &regex::Regex::new(r"\\b\\w+\\b").unwrap(), &regex::Regex::new(r".*").unwrap()), &regex::Regex::new(r"\\b\\w+\\b").unwrap());
    /// assert_eq!(re_for_kind("field", &regex::Regex::new(r".*").unwrap(), &regex::Regex::new(r"\\b\\w+\\b").unwrap(), &regex::Regex::new(r".*").unwrap()), &regex::Regex::new(r".*").unwrap());
    /// assert_eq!(re_for_kind("other", &regex::Regex::new(r".*").unwrap(), &regex::Regex::new(r"\\b\\w+\\b").unwrap(), &regex::Regex::new(r".*").unwrap()), &regex::Regex::new(r".*").unwrap());
    ///
    /// ```
    #[inline]
    fn re_for_kind<'a>(
        kind: &str,
        re_fn: &'a Regex,
        re_struct: &'a Regex,
        re_field: &'a Regex,
    ) -> &'a Regex {
        match kind {
            "struct" => re_struct,
            "field" => re_field,
            _ => re_fn,
        }
    }

    // Prefix every "///" line in `doc` with `indent`.
    /// Indents a Rustdoc comment block by adding indentation to each line.
    ///
    /// This function takes a string representing the original Rustdoc comment
    /// and an indentation string, then returns a new version of the doc with
    /// each line indented by the specified amount.
    ///
    /// Parameters:
    /// - `doc`: The original Rustdoc comment as a string slice.
    /// - `indent`: The indentation to add to each line of the doc.
    ///
    /// Returns:
    /// A new string with all lines indented by `indent`.
    ///
    /// Notes:
    /// The function preserves the structure of the original doc, including
    /// special markers like `///` that indicate comment lines.
    ///
    /// Examples:
    /// ```rust
    /// let doc = "/// Hello, world!
    /// This is a test.";
    /// let indented_doc = indent_rustdoc(doc, "    ");
    /// assert_eq!(indented_doc, "    /// Hello, world!
    /// This is a test.");
    ///
    /// ```
    fn indent_rustdoc(doc: &str, indent: &str) -> String {
        let mut out = String::with_capacity(doc.len() + indent.len() * 4);
        for (i, line) in doc.lines().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if line.starts_with("///") {
                out.push_str(indent);
                out.push_str(line);
            } else {
                // Paranoid fallback: ensure we still emit a doc line
                out.push_str(indent);
                out.push_str("/// ");
                out.push_str(line);
            }
        }
        out
    }

    // Bucket results by file
    let mut by_file: BTreeMap<&str, Vec<&LlmDocResult>> = BTreeMap::new();
    for r in results {
        by_file.entry(&r.file).or_default().push(r);
    }

    for (file, items) in by_file {
        let original = fs::read_to_string(file).with_context(|| format!("reading {}", file))?;

        // Precompute line start byte offsets for [line -> byte] mapping
        let mut line_starts: Vec<usize> = vec![0];
        for (i, b) in original.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        line_starts.push(original.len()); // sentinel

        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut skipped_no_sig = 0usize;
        let mut skipped_existing_doc = 0usize;

        // Apply later edits first: walk items descending by start line
        for r in items
            .iter()
            .sorted_by_key(|r| r.start_line.unwrap_or(0))
            .rev()
        {
            let Some(start_line_1) = r.start_line else {
                continue;
            };
            let start_line0 = start_line_1.saturating_sub(1) as usize;

            // Choose signature locator for this kind
            let re_item = re_for_kind(&r.kind, &re_fn, &re_struct, &re_field);

            // Find the item signature line if applicable
            let sig_line0_opt = find_item_sig_line_near(&original, start_line0, re_item);

            // For fields, we allow missing signature: insertion anchor is start_line0
            let (ins_lo, ins_hi, indent_line_idx) = match (r.kind.as_str(), sig_line0_opt) {
                ("struct", Some(sig_line0)) => {
                    let (lo, hi) = match struct_doc_insertion_range_above_attrs(
                        &original, sig_line0, overwrite,
                    ) {
                        Some(pair) => pair,
                        None => {
                            skipped_existing_doc += 1;
                            continue;
                        }
                    };
                    // Indent with the first attribute line (hi), or the struct line if no attrs.
                    (lo, hi, hi.min(sig_line0))
                }
                ("field", _) => {
                    let (lo, hi) =
                        match field_doc_insertion_range(&original, start_line0, overwrite) {
                            Some(pair) => pair,
                            None => {
                                skipped_existing_doc += 1;
                                continue;
                            }
                        };
                    // Indent with the actual field line (hi)
                    (lo, hi, hi)
                }
                // functions & everything else treated as fn
                (_, Some(sig_line0)) => {
                    // Use your existing function logic to place docs (keeps one blank above if present)
                    let (lo, hi) = find_doc_insertion_range(&original, sig_line0 + 1);
                    (lo, hi, sig_line0)
                }
                // If we cannot find a signature for non-field items, skip
                _ => {
                    skipped_no_sig += 1;
                    continue;
                }
            };

            // Respect overwrite only if we're replacing an existing doc block.
            // (Non-empty ranges can also be just whitespace that we intend to trim.)
            let has_doc_block_in_range = {
                let lines: Vec<&str> = original.lines().collect();
                let lo = ins_lo.min(lines.len());
                let hi = ins_hi.min(lines.len());
                (lo..hi).any(|k| {
                    let t = lines[k].trim_start();
                    t.starts_with("///") || t.starts_with("#![doc") || t.starts_with("#[doc")
                })
            };

            if !overwrite && has_doc_block_in_range {
                skipped_existing_doc += 1;
                continue;
            }

            // Compute byte range [start_b, end_b)
            let start_b = *line_starts.get(ins_lo).unwrap_or(&0);
            let end_b = *line_starts.get(ins_hi).unwrap_or(&start_b);

            // Prepare replacement, **indented to match the target line**
            let mut repl = r.llm_doc.clone();

            // Determine indentation from the target line we're inserting above
            let target_line = original.lines().nth(indent_line_idx).unwrap_or("");
            let indent: String = target_line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            if !indent.is_empty() {
                repl = indent_rustdoc(&repl, &indent);
            }

            if !repl.ends_with('\n') {
                repl.push('\n');
            }

            edits.push((start_b, end_b, repl));
        }

        // Apply edits from bottom to top to keep earlier byte offsets valid
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        if edits.is_empty() {
            eprintln!(
                "Patched {}: 0 edits (skipped_no_sig={}, skipped_existing_doc={})",
                file, skipped_no_sig, skipped_existing_doc
            );
            continue;
        }

        let mut text = original;
        for (start_b, end_b, repl) in edits {
            if start_b <= end_b && end_b <= text.len() {
                text.replace_range(start_b..end_b, &repl);
            }
        }

        fs::write(file, text).with_context(|| format!("writing {}", file))?;
    }

    Ok(())
}

/// Handle a function to find the insertion range for documentation lines in a source string.
///
/// This function identifies and returns the start (`lo`) and end (`hi`)
/// indices within a source string, corresponding to the insertion range
/// for documentation lines (e.g., `///` or `#[doc]`) immediately before a function signature.
///
/// Parameters:
/// - `source`: The input string containing code and documentation lines.
/// - `start_line_1`: The index of the first line in the signature (1-based).
///
/// Returns:
/// - A tuple `(lo, hi)` representing the range of lines to preserve for documentation
///   insertion. `lo` is typically above or within the signature, and `hi` marks
///   where to end insertion (e.g., before attributes).
///
/// Safety:
/// - This function assumes input is well-formed and may panic on invalid data.
///
/// Notes:
/// - It handles cases with both traditional `///` comments and modern document
///   attributes like `#[doc]`.
/// - It ensures no extra blank lines are left between documentation and
///   attributes.
/// - `usize::MAX` is treated as a sentinel value for line indexing.
///
/// Examples:
/// ```rust
/// let src = "/// This is a doc comment
/// fn example() { ... }";
/// assert_eq!(find_doc_insertion_range(src, 1), (0, 2));
/// ```
/// ```rust
/// let src = "fn example() { ... }
/// Another doc comment";
/// assert_eq!(find_doc_insertion_range(src, 1), (2, 4));
///
/// ```
fn find_doc_insertion_range(source: &str, start_line_1: usize) -> (usize, usize) {
    let lines: Vec<&str> = source.lines().collect();
    let sig_idx = start_line_1.saturating_sub(1);
    let mut lo = sig_idx;

    // Walk upward over existing doc lines (/// or #[doc] variants)
    let mut i = sig_idx.saturating_sub(1);
    while i < lines.len() {
        if i == usize::MAX {
            break;
        }
        let t = lines[i].trim_start();
        if t.starts_with("///") || t.starts_with("#![doc") || t.starts_with("#[doc") {
            lo = i;
            if i == 0 {
                break;
            }
            i = i.saturating_sub(1);
            continue;
        }
        break;
    }

    // Walk upward over an attribute block immediately above the item.
    // Return insertion range that ends at the FIRST attribute line,
    // so there is NO blank line left between doc and attribute.
    let mut j = sig_idx.saturating_sub(1);
    let mut saw_attr = false;
    let mut attr_first_idx = usize::MAX;

    while j < lines.len() {
        if j == usize::MAX {
            break;
        }
        let t = lines[j].trim_start();
        if t.starts_with("#[") || t.starts_with("#![") {
            saw_attr = true;
            attr_first_idx = j; // keep the top-most attribute line
            lo = lo.min(j);
            if j == 0 {
                break;
            }
            j = j.saturating_sub(1);
            continue;
        }
        break;
    }

    if saw_attr {
        // If the line above the attribute is blank AND currently included, pull `lo` up to the attribute
        // and end the replacement *at* the attribute, removing the blank separator entirely.
        // (No extra blank line between doc and attribute.)
        let hi = attr_first_idx;
        return (lo, hi);
    } else {
        // No attributes: include exactly one blank above the signature if present
        let mut lo2 = lo;
        if sig_idx > 0 && lines[sig_idx - 1].trim().is_empty() {
            lo2 = lo2.min(sig_idx - 1);
        }
        return (lo2, sig_idx);
    }
}

// ------------------- Sanitization ------------------------------------------

/// Best-effort sanitizer that extracts a clean `///` rustdoc block from arbitrary model output,
/// ensuring proper formatting and structure for documentation purposes.
///
/// Parameters:
/// - `raw`: A string slice containing the raw text to sanitize, often from an LLM model.
///
/// Returns:
/// - A `String` representing the cleaned and formatted rustdoc block, or an empty string if no valid doc was found.
///
/// Errors:
/// - This function does not return explicit errors; it simply returns an empty string if no valid doc is found.
///
/// Notes:
/// - The function attempts to strip wrappers, de-escape sequences, normalize line endings,
///   and enforce proper documentation formatting.
/// - It ensures every line starts with `///`, removes trailing backslashes,
///   and balances doctest fences for compatibility.
///
/// Examples:
/// ```rust
/// let raw = "   ```rust
/// This is a test
/// assert_eq!(sanitize_llm_doc(&raw), "/// This is a test");
///
/// ```
fn sanitize_llm_doc(raw: &str) -> String {
    // 1) Strip obvious wrappers like <think>…</think> and outer ``` fences
    let s = strip_wrappers_and_fences_strict(raw);

    // 2) De-escape common sequences the model may have double-escaped inside JSON
    let s = decode_common_escapes(&s);

    // 3) Normalize CRLF and trim right edges
    let s = s.replace('\r', "");
    let mut lines: Vec<String> = s.lines().map(|l| l.trim_end().to_string()).collect();

    // 4) If nothing useful, bail
    if lines.iter().all(|l| l.trim().is_empty()) {
        return String::new();
    }

    // 5) Convert any obvious headings the prompt allows into canonical text (your original rule)
    for l in &mut lines {
        match l.trim() {
            "Parameters:" => *l = "## Parameters".into(),
            "Returns:" => *l = "## Returns".into(),
            "Errors:" => *l = "## Errors".into(),
            "Safety:" => *l = "## Safety".into(),
            "Notes:" => *l = "## Notes".into(),
            "Examples:" => *l = "## Examples".into(),
            _ => {}
        }
    }

    // 6) Coerce every line to a `///` line (collapsing multiple blanks), but DO NOT keep
    //    non-doc garbage like JSON braces/keys. We'll post-filter to the longest contiguous
    //    block of `///` lines.
    let mut coerced: Vec<String> = Vec::with_capacity(lines.len());
    let mut prev_blank = false;
    for l in lines {
        let mut t = l.trim().to_string();
        // Drop naked fence wrappers that sometimes appear
        if t.starts_with("```") && !t.starts_with("///") {
            continue;
        }
        let is_blank = t.is_empty();
        if is_blank {
            if prev_blank {
                continue;
            }
            prev_blank = true;
            coerced.push("///".into());
            continue;
        }
        prev_blank = false;

        // If line already starts with `///`, keep as-is
        if t.starts_with("///") {
            coerced.push(t);
            continue;
        }

        // Drop obvious JSON keys / braces / commas that sneak through
        if t == "{" || t == "}" || t == "}," || t.ends_with(":") || t.ends_with("\":") {
            continue;
        }

        // If line is quoted like: "/// something" or "some text",
        // strip leading/trailing quotes once.
        if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
            t = t[1..t.len() - 1].to_string();
        }

        // Finally force to rustdoc
        coerced.push(format!("/// {}", t));
    }

    // 7) Keep the *longest contiguous* block of `///` lines
    let doc_block = extract_longest_doc_block(&coerced);

    // 8) Balance doctest fences and default to ```rust when model left it bare
    let mut out: Vec<String> = Vec::with_capacity(doc_block.len());
    let mut fence_depth = 0usize;
    for mut l in doc_block {
        // Clean trailing backslashes that the model sometimes adds to indicate soft-wraps
        // e.g., `/// …text...\`
        if l.ends_with('\\') && !l.ends_with("\\\\") {
            l.pop();
        }

        // Convert bare ``` to ```rust for doctest friendliness
        let t = l.trim_start_matches('/').trim_start();
        if t.starts_with("```") {
            if fence_depth == 0 && t == "```" {
                l = "/// ```rust".into();
            }
            fence_depth ^= 1;
        }
        out.push(l);
    }
    if fence_depth == 1 {
        out.push("/// ```".into());
    }

    // 9) Trim trailing blank doc lines
    while matches!(out.last().map(|s| s.trim_end()), Some("///") | Some("")) {
        out.pop();
    }

    // 10) Guarantee everything starts with `///` and drop initial blank `///` runs
    let joined = out
        .into_iter()
        .map(|line| {
            if line.starts_with("///") {
                line
            } else {
                format!("/// {}", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    strip_leading_empty_doc_lines(&joined)
}

/// Decode common escape sequences from a string, particularly useful when processing text returned by language models.
///
/// This function heuristically decodes common escape sequences such as `
/// ` (newline), `\	` (tab), and `"` (double quote) from a string.
/// It ensures that escaped backslash-newline pairs are decoded first, followed by other common escapes.
///
/// Parameters:
/// - `s`: The input string containing escape sequences to be decoded.
///
/// Returns:
/// A new `String` with the common escape sequences decoded.
///
/// Notes:
/// - The order of replacements is important to ensure correct decoding.
/// - This function is commonly used in contexts where language models return text within JSON strings.
///
/// Examples:
/// ```rust
/// let input = r"Hello\\
/// World";
/// let result = decode_common_escapes(input);
/// assert_eq!(result, "Hello
/// World");
///
/// let input = r"Hello"World";
/// let result = decode_common_escapes(input);
/// assert_eq!(result, "Hello"World");
///
/// let input = r"Hello\
/// World";
/// let result = decode_common_escapes(input);
/// assert_eq!(result, "Hello
/// World");
///
/// ```
fn decode_common_escapes(s: &str) -> String {
    let mut t = s.to_string();
    // Order matters: decode escaped backslash-newline pairs first
    t = t.replace("\\r\\n", "\n");
    t = t.replace("\\n", "\n");
    t = t.replace("\\t", "\t");
    t = t.replace("\\\"", "\"");
    // Sometimes we get doubled escaping: `\\\\n` → `\n`
    t = t.replace("\\\\n", "\n");
    t = t.replace("\\\\t", "\t");
    t = t.replace("\\\\\"", "\"");
    t
}

/// Extracts the longest contiguous documentation block (`/// ...`) from a list of strings.
///
/// Looks for lines starting with `///` (with optional whitespace), and returns the
/// longest continuous region of such lines. If no full block is found, it defaults
/// to wrapping the first non-empty line with `///`.
///
/// Parameters:
/// - `lines`: A slice of strings to search for documentation blocks.
///
/// Returns:
/// - A `Vec<String>` containing the longest contiguous documentation block found,
///   or a single line wrapped in `///` if no full block is detected.
///
/// Errors:
/// - None are returned; all errors are handled internally and no `Result` is used.
///
/// Notes:
/// - This function is typically used to extract documentation from a list of lines,
///   often as part of parsing code or configuration files.
///
/// Examples:
/// ```rust
/// let lines = vec![
///     "Some normal line",
///     "   /// This is a doc block.",
///     "   /// Another line of the same block.",
///     "",
/// ];
///
/// let doc_block = extract_longest_doc_block(&lines);
/// assert_eq!(doc_block, vec!["   /// This is a doc block.", "   /// Another line of the same block."]);
/// ```
///
/// ```rust
/// let lines = vec!["   line1", "   line2"];
/// let doc_block = extract_longest_doc_block(&lines);
/// assert_eq!(doc_block, vec!["   line1", "   line2"]);
/// ```
///
/// ```rust
/// let lines = vec!["line1", "   /// line2"];
/// let doc_block = extract_longest_doc_block(&lines);
/// assert_eq!(doc_block, vec!["   /// line2"]);
///
/// ```
fn extract_longest_doc_block(lines: &[String]) -> Vec<String> {
    let mut best_start = 0usize;
    let mut best_len = 0usize;

    let mut cur_start = None::<usize>;
    let mut cur_len = 0usize;

    let is_doc = |s: &str| s.trim_start().starts_with("///");
    for (i, l) in lines.iter().enumerate() {
        if is_doc(l) {
            if cur_start.is_none() {
                cur_start = Some(i);
                cur_len = 0;
            }
            cur_len += 1;
        } else if let Some(st) = cur_start {
            if cur_len > best_len {
                best_len = cur_len;
                best_start = st;
            }
            cur_start = None;
            cur_len = 0;
        }
    }
    if let Some(st) = cur_start {
        if cur_len > best_len {
            best_len = cur_len;
            best_start = st;
        }
    }

    if best_len == 0 {
        // Fallback: make a single-line doc from the first nonempty line
        let first = lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        return vec![if first.starts_with("///") {
            first
        } else {
            format!("/// {}", first)
        }];
    }

    lines[best_start..best_start + best_len].to_vec()
}

/// Strips leading empty documentation lines from a string.
/// This function processes the input `&str` to remove all leading lines that are empty or consist of only "///" followed by optional whitespace.
///
/// Parameters:
/// - `s`: The input string to process, containing potential leading doc lines.
///
/// Returns:
/// - A `String` with the leading empty documentation lines removed.
///
/// Errors:
/// - None; this function does not return an error.
///
/// Notes:
/// - The function treats lines with only "///" (followed by optional whitespace) as empty.
/// - The input string is processed in a non-destructive manner, preserving its original content except for the leading lines.
///
/// Examples:
/// ```no_run
/// let input = "    ///
///
///Hello, world!";
/// assert_eq!(strip_leading_empty_doc_lines(input), "Hello, world!");
///
/// ```
fn strip_leading_empty_doc_lines(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() && lines[i].trim_end() == "///" {
        i += 1;
    }
    if i == 0 {
        return s.to_string();
    }
    lines[i..].join("\n")
}

/// Strips wrappers and fences from a string, preserving only the inner content.
///
/// This function processes a given string to remove common wrapper markers and
/// fenced code blocks, returning the cleaned content. It handles various cases,
/// including normal text, rustdoc-style comments, and fenced code blocks.
///
/// Parameters:
/// - `s`: The input string to process.
///
/// Returns:
/// A new `String` containing the processed content, with wrappers and fences removed.
///
/// Notes:
/// - It handles both standard text and rustdoc-style comments.
/// - Fenced code blocks are stripped, with special handling for rustdoc fences
///   (e.g., "/// ```") to avoid truncating content inside them.
///
/// Examples:
/// ```no_run
/// let input = r#"
/// ```rust
/// fn main() {
///     println!("Hello, world!");
/// }
/// ```rust
/// "#;
///
/// let result = strip_wrappers_and_fences_strict(input);
/// assert_eq!(result, "fn main() {
/// println!("Hello, world!);");
/// }");
///
/// ```
fn strip_wrappers_and_fences_strict(s: &str) -> String {
    let mut t = s.trim().to_string();

    // Drop <think>…</think>
    let re_think = regex::Regex::new(r"(?is)<\s*think(?:\s+[^>]*)?>.*?</\s*think\s*>").unwrap();
    t = re_think.replace_all(&t, "").to_string();
    t = t.trim().to_string();

    // If it already looks like a rustdoc block, don't try to "unwrap" markers:
    // (prevents truncation when tokens like `ANSWER:` appear inside prose or code)
    let doc_line_count = t
        .lines()
        .filter(|l| l.trim_start().starts_with("///"))
        .take(3)
        .count();
    let looks_like_rustdoc = doc_line_count >= 3;

    // Remove common wrapper markers ONLY if they appear at the start of a line,
    // outside of fenced code blocks, and ONLY if it doesn't already look like rustdoc.
    if !looks_like_rustdoc {
        let markers = ["ANSWER:", "RESPONSE:", "OUTPUT:", "QUESTION:"];

        // Walk lines, tracking fences; if we see a marker at line start (after trim),
        // keep everything AFTER that marker.
        let mut in_fence = false;
        let mut split_byte_index: Option<usize> = None;

        // Precompute cumulative byte starts of each line for precise slicing.
        let mut byte_pos = 0usize;
        for line in t.lines() {
            let trimmed = line.trim_start();

            if trimmed.starts_with("```") {
                in_fence = !in_fence;
            }

            if !in_fence {
                if let Some(m) = markers.iter().find(|m| trimmed.starts_with(**m)) {
                    // Position where this line's left-trim begins
                    let left_trim_offset = line.len() - trimmed.len();
                    // Slice AFTER the marker
                    split_byte_index = Some(byte_pos + left_trim_offset + m.len());
                }
            }

            byte_pos += line.len();
            // add the newline byte if there is one
            if byte_pos < t.len() {
                byte_pos += 1;
            }
        }

        if let Some(idx) = split_byte_index {
            t = t[idx..].trim().to_string();
        }
    }

    // If the *entire* payload is a single fenced block, keep only the interior.
    // (This is safe even for rustdoc blocks, because rustdoc fences are "/// ```",
    // not a naked "```" as first/last lines.)
    let mut fence_count = 0usize;
    let mut lines = Vec::new();
    for line in t.lines() {
        let l = line.trim_end();
        if l.starts_with("```") {
            fence_count += 1;
        }
        lines.push(l);
    }
    if fence_count == 2
        && lines.first().map(|l| l.starts_with("```")).unwrap_or(false)
        && lines.last().map(|l| l.starts_with("```")).unwrap_or(false)
    {
        return lines[1..lines.len() - 1].join("\n");
    }

    // Strip stray leading/trailing backticks (not fences), just in case.
    t.trim_matches('`').trim().to_string()
}
