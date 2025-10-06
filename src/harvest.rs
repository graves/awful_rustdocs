use crate::error::{Error, Result};
use crate::model::Row;
use crate::runner::{ProcRunner, ToolRunner};
use std::path::{Path, PathBuf};

fn shell_escape(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}
fn shell_escape_lossy_path(p: &Path) -> String {
    shell_escape(&p.to_string_lossy())
}

pub fn run_nushell_harvest(script_path: &PathBuf, targets: &[PathBuf]) -> Result<Vec<Row>> {
    let mut call = format!("source {}; let rows = (rust-ast", shell_escape_lossy_path(script_path));
    if targets.is_empty() {
        call.push_str(" .");
    } else {
        for t in targets {
            call.push(' ');
            call.push_str(&shell_escape_lossy_path(t));
        }
    }
    call.push_str("); $rows | to json");

    let runner = ProcRunner;
    let stdout = runner.run_text("nu", &["--no-config-file", "-c", &call])?;

    let rows: Vec<Row> =
        serde_json::from_str(&stdout).map_err(|e| Error::Json { context: "nu rust-ast JSON", source: e })?;
    Ok(rows)
}
