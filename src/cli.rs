use clap::{ArgAction, Parser, Subcommand};
use directories::ProjectDirs;
use std::{fs, path::{Path, PathBuf}};
use crate::error::{Error, Result};
use crate::defaults::{DEFAULT_CONFIG_YAML, DEFAULT_RUSTDOC_FN_YAML, DEFAULT_RUSTDOC_STRUCT_YAML};

#[derive(Parser, Debug)]
#[command(
    name = "awful_rustdocs",
    about = "Generate rustdocs for functions and structs using Awful Jade + rust_ast.nu"
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Init {
        #[arg(long, action=ArgAction::SetTrue)]
        force: bool,
        #[arg(long, action=ArgAction::SetTrue)]
        dry_run: bool,
    },
    Run(GenerateOpts),
}

#[derive(Debug, clap::Args, Clone)]
pub struct GenerateOpts {
    #[arg(long, default_value = "rust_ast.nu")]
    pub script: PathBuf,
    #[arg()]
    pub targets: Vec<PathBuf>,
    #[arg(long, action=ArgAction::SetTrue)]
    pub write: bool,
    #[arg(long, action=ArgAction::SetTrue)]
    pub overwrite: bool,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, action=ArgAction::SetTrue)]
    pub no_calls: bool,
    #[arg(long, action=ArgAction::SetTrue)]
    pub no_paths: bool,
    #[arg(long, default_value = "rustdoc_fn")]
    pub fn_template: String,
    #[arg(long, default_value = "rustdoc_struct")]
    pub struct_template: String,
    #[arg(long, default_value = "rustdoc_config.yaml")]
    pub config: String,
    #[arg(long = "only", value_delimiter = ',', value_name = "SYMBOL", num_args=1..)]
    pub only: Vec<String>,
}

pub fn config_root() -> Result<PathBuf> {
    let proj = ProjectDirs::from("com", "awful-sec", "aj").ok_or(Error::ConfigDirUnavailable)?;
    Ok(proj.config_dir().to_path_buf())
}

fn write_if_needed(path: &Path, contents: &str, force: bool) -> Result<bool> {
    if path.exists() && !force { return Ok(false); }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::Io { path: Some(parent.to_path_buf()), source: e })?;
    }
    fs::write(path, contents).map_err(|e| Error::Io { path: Some(path.to_path_buf()), source: e })?;
    Ok(true)
}

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
    eprintln!("{} {}", if w3 { "Wrote" } else { "Kept" }, struct_tpl.display());
    Ok(())
}
