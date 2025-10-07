use crate::defaults::{DEFAULT_CONFIG_YAML, DEFAULT_RUSTDOC_FN_YAML, DEFAULT_RUSTDOC_STRUCT_YAML};
use crate::error::{Error, Result};
use clap::{ArgAction, Parser, Subcommand};
use directories::ProjectDirs;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser, Debug)]
#[command(
    name = "awful_rustdocs",
    about = "Generate rustdocs for functions and structs using Awful Jade + rust_ast.nu"
)]

/// The CLI entry point, containing a subcommand to execute.
pub struct Cli {
    /// Subcommand to execute (e.g., `help`, `version`, `run`).
    #[command(subcommand)]
    pub cmd: Command,
}

/// Enumerates the commands Clap expects.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize the application's configuration files.
    Init {
        /// Overwrite the previous configuration if it exists.
        #[arg(long, action=ArgAction::SetTrue)]
        force: bool,
        /// Print the filepaths of the configuration that will be created.
        #[arg(long, action=ArgAction::SetTrue)]
        dry_run: bool,
    },
    // Run the application.
    Run(GenerateOpts),
}

/// Configuration options for generating documentation from a script.
#[derive(Debug, clap::Args, Clone)]
pub struct GenerateOpts {
    /// Script file to process, default is "rust_ast.nu".
    #[arg(long, default_value = "rust_ast.nu")]
    pub script: PathBuf,
    /// List of target paths to generate documentation for.
    #[arg()]
    pub targets: Vec<PathBuf>,
    /// If set, write generated output to files.
    #[arg(long, action=ArgAction::SetTrue)]
    pub write: bool,
    /// If set, overwrite existing files without prompting.
    #[arg(long, action=ArgAction::SetTrue)]
    pub overwrite: bool,
    /// Session identifier to use for state persistence.
    #[arg(long)]
    pub session: Option<String>,
    /// Maximum number of items to process; if None, no limit.
    #[arg(long)]
    pub limit: Option<usize>,
    /// If set, skip function call generation.
    #[arg(long, action=ArgAction::SetTrue)]
    pub no_calls: bool,
    /// If set, skip path generation.
    #[arg(long, action=ArgAction::SetTrue)]
    pub no_paths: bool,
    /// Template to use for function definitions, default is "rustdoc_fn".
    #[arg(long, default_value = "rustdoc_fn")]
    pub fn_template: String,
    /// Template to use for struct definitions, default is "rustdoc_struct".
    #[arg(long, default_value = "rustdoc_struct")]
    pub struct_template: String,
    /// Configuration file path, default is "rustdoc_config.yaml".
    #[arg(long, default_value = "rustdoc_config.yaml")]
    pub config: String,
    /// Comma-separated list of symbols to generate documentation for only.
    #[arg(long = "only", value_delimiter = ',', value_name = "SYMBOL", num_args=1..)]
    pub only: Vec<String>,
}

/// Returns the path to the root configuration directory for the AwfulJade application.
///
/// This function uses `ProjectDirs` to determine the standard application configuration directory
/// on the user's system, following platform-specific conventions. If the directory is not available,
/// it returns an error indicating that the configuration directory is unavailable.
///
/// # Returns
/// - A `Result<PathBuf>` containing the path to the config root directory on success.
///
/// # Errors
/// - `Error::ConfigDirUnavailable` if the configuration directory cannot be determined (e.g., due to missing or inaccessible system directories).
///
/// # Notes
/// - The directory follows the XDG Base Directory Specification on Unix-like systems and Windows standards.
/// - The path is derived from the application's vendor ("com"), application ("awful-sec"), and name ("aj").
pub fn config_root() -> Result<PathBuf> {
    let proj = ProjectDirs::from("com", "awful-sec", "aj").ok_or(Error::ConfigDirUnavailable)?;
    Ok(proj.config_dir().to_path_buf())
}

/// Writes content to a file if the file does not exist or if `force` is `true`.
/// If the file exists and `force` is `false`, the function returns `Ok(false)` without writing.
/// It ensures the parent directory exists before writing, and handles I/O errors with descriptive context.
///
/// # Parameters
/// - `path`: Path to the file to write to.
/// - `contents`: The string content to write to the file.
/// - `force`: If `true`, overwrites the file even if it exists; otherwise, skips if the file already exists.
///
/// # Returns
/// - `Ok(true)` if the file was written successfully.
/// - `Ok(false)` if the file already exists and `force` is `false`.
///
/// # Errors
/// - `Error::Io` if there is an I/O error during directory creation or file writing, including permission issues or disk full errors.
/// - The error includes the path that failed and the underlying cause.
///
/// # Notes
/// - The function creates parent directories recursively if they do not exist.
/// - It does not validate the content or path format.
/// - The function is safe to use in concurrent contexts as it only performs file operations.
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use crate::Error;
/// let path = Path::new("test/output.txt");
///
/// let contents = "Hello, world!";
/// assert!(write_if_needed(path, contents, false).unwrap());
/// ```
fn write_if_needed(path: &Path, contents: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::Io {
            path: Some(parent.to_path_buf()),
            source: e,
        })?;
    }
    fs::write(path, contents).map_err(|e| Error::Io {
        path: Some(path.to_path_buf()),
        source: e,
    })?;
    Ok(true)
}

/// Initializes the configuration and template directory for the Rustdoc tool by creating or loading default configuration and template files. If `dry_run` is true, it prints the files that would be created without actually writing them. On success, it logs whether each file was written or kept based on the `force` flag.
///
/// Parameters:
/// - `force`: If `true`, overwrites existing configuration and template files regardless of their current state.
/// - `dry_run`: If `true`, simulates the operation and prints the files that would be created without writing them.
///
/// Returns:
/// - `Ok(())` on successful initialization or if no changes were made.
/// - `Err` if any I/O operations fail (e.g., file system errors).
///
/// Errors:
/// - I/O errors when reading, writing, or creating files.
/// - Errors during file system path resolution.
///
/// Notes:
/// - Creates or updates three files: `rustdoc_config.yaml`, `rustdoc_fn.yaml`, and `rustdoc_struct.yaml` in the config directory.
/// - The configuration directory is determined by `config_root()`, which resolves to a user-specific or default location.
/// - If `force` is false and files already exist, they are not overwritten.
pub fn run_init(force: bool, dry_run: bool) -> Result<()> {
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
