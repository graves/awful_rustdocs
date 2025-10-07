use crate::error::{Error, Result};
use std::process::Command as ProcCommand;

pub trait ToolRunner {
    /// Processes a JSON Lines input stream by invoking a specified tool with given arguments.
    /// The function parses each line of the input as a JSON object, extracts the relevant fields,
    /// and invokes the named tool with the provided arguments. The results are collected into a vector
    /// of strings and returned. This function is intended to be used in a pipeline where JSON Lines
    /// data is processed sequentially.
    ///
    /// Parameters:
    /// - `tool`: A static string slice identifying the tool to invoke.
    /// - `args`: A slice of string slices representing arguments to pass to the tool.
    ///
    /// Returns:
    /// - A `Result<Vec<String>>` containing the output lines from the tool invocation,
    ///   or an error if processing fails.
    ///
    /// Errors:
    /// - Returns errors from JSON parsing of input lines.
    /// - Returns errors from tool invocation failures.
    /// - Returns I/O errors when reading or writing input/output streams.
    ///
    /// Notes:
    /// - The input must be valid JSON Lines format, with each line being a valid JSON object.
    /// - The tool must be registered and available in the system for invocation.
    /// - This function does not validate or sanitize input arguments.
    fn run_json_lines(&self, tool: &'static str, args: &[&str]) -> Result<Vec<String>>;

    /// Runs a text-based operation using a specified tool and arguments.
    /// Invokes the given tool with the provided arguments and returns the resulting output as a string.
    /// This function is intended for internal use within the runner and does not expose direct interaction with external tools.
    ///
    /// Parameters:
    /// - `tool`: A static string slice identifying the tool to execute.
    /// - `args`: A slice of string slices representing the arguments to pass to the tool.
    ///
    /// Returns:
    /// - A `Result<String>` containing the output of the tool execution on success, or an error otherwise.
    ///
    /// Errors:
    /// - Returns errors if the tool is not recognized, arguments are malformed, or the execution fails.
    ///
    /// Notes:
    /// - The tool must be defined at compile time via a `&'static str`.
    /// - Arguments are passed directly to the tool with no parsing or validation.
    /// - This function does not perform any I/O beyond the tool's execution.
    fn run_text(&self, tool: &'static str, args: &[&str]) -> Result<String>;
}

/// A runner for executing tools via system processes, supporting both JSON lines and text output modes.
pub struct ProcRunner;

impl ToolRunner for ProcRunner {
    /// Runs a tool via a shell command and returns its output as a vector of strings, parsing JSON lines from stdout.
    /// The function spawns a subprocess using `ProcCommand`, executes it with the provided tool name and arguments,
    /// and parses the output line by line, filtering out empty lines. If the command fails or exits with a non-zero status,
    /// an error is returned with relevant context including the exit code and stderr.
    ///
    /// # Parameters
    /// - `tool`: A static string slice representing the name of the tool to execute (e.g., "ast-grep", "rust-ast").
    /// - `args`: A slice of string slices representing the command-line arguments to pass to the tool.
    ///
    /// # Returns
    /// - A `Result<Vec<String>>` containing lines from the stdout of the executed tool, each stripped of leading/trailing whitespace and empty lines, or an error if the command fails.
    ///
    /// # Errors
    /// - `Error::ToolSpawn` if the tool fails to spawn (e.g., due to missing executable or permission issues).
    /// - `Error::ToolStatus` if the tool exits with a non-zero status, including stderr content as a hint.
    ///
    /// # Notes
    /// - The output is parsed as lines, trimmed, and only non-empty lines are included in the result.
    /// - This function assumes the tool produces valid UTF-8 output.
    /// - The output is not guaranteed to be JSON; it is expected to be JSON lines (one JSON object per line).
    fn run_json_lines(&self, tool: &'static str, args: &[&str]) -> Result<Vec<String>> {
        let out = ProcCommand::new(tool)
            .args(args)
            .output()
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

    /// Runs a text-processing command using an external tool via `ProcCommand`.
    ///
    /// Parameters:
    /// - `tool`: The name of the external tool to execute (e.g., "curl", "grep").
    /// - `args`: A slice of string slices representing the arguments to pass to the tool.
    ///
    /// Returns:
    /// - A `Result<String>` containing the stdout output of the executed tool, if successful.
    ///
    /// Errors:
    /// - `Error::ToolSpawn` if the tool fails to spawn (e.g., due to missing binary or permissions).
    /// - `Error::ToolStatus` if the tool exits with a non-zero status, including details from stderr.
    ///
    /// Notes:
    /// - The function handles UTF-8 decoding of stdout and stderr, and only returns valid UTF-8 strings.
    /// - If the tool fails, the error includes a hint from stderr to aid debugging.
    /// - The command is executed in a shell-like environment using `ProcCommand`.
    fn run_text(&self, tool: &'static str, args: &[&str]) -> Result<String> {
        let out = ProcCommand::new(tool)
            .args(args)
            .output()
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
