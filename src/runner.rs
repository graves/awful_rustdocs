use crate::error::{Error, Result};
use std::process::Command as ProcCommand;

pub trait ToolRunner {
    fn run_json_lines(&self, tool: &'static str, args: &[&str]) -> Result<Vec<String>>;
    fn run_text(&self, tool: &'static str, args: &[&str]) -> Result<String>;
}

pub struct ProcRunner;

impl ToolRunner for ProcRunner {
    fn run_json_lines(&self, tool: &'static str, args: &[&str]) -> Result<Vec<String>> {
        let out = ProcCommand::new(tool).args(args).output()
            .map_err(|e| Error::ToolSpawn { tool, source: e })?;
        if !out.status.success() {
            return Err(Error::ToolStatus {
                tool,
                code: out.status.code(),
                stderr_hint: Some(String::from_utf8_lossy(&out.stderr).into()),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    fn run_text(&self, tool: &'static str, args: &[&str]) -> Result<String> {
        let out = ProcCommand::new(tool).args(args).output()
            .map_err(|e| Error::ToolSpawn { tool, source: e })?;
        if !out.status.success() {
            return Err(Error::ToolStatus {
                tool,
                code: out.status.code(),
                stderr_hint: Some(String::from_utf8_lossy(&out.stderr).into()),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
