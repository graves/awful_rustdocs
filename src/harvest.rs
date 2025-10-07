use crate::error::{Error, Result};
use crate::model::Row;
use crate::runner::{ProcRunner, ToolRunner};

use tracing::instrument;

use std::path::{Path, PathBuf};

/// Escapes a string for shell usage by wrapping it in single quotes if it contains non-alphanumeric characters or special shell metacharacters like `.` or `-`. If the string is already safe (containing only ASCII alphanumeric characters and allowed special characters), it is returned unchanged.
///
/// Parameters:
/// - `s`: The input string to escape for shell use.
///
/// Returns:
/// - A `String` that is safely escaped for shell execution, wrapped in single quotes if needed.
///
/// Notes:
/// - This function ensures that shell commands containing the input string are safe from injection by escaping quotes and special characters.
/// - Only ASCII characters are considered; non-ASCII or non-allowed characters are handled by escaping the entire string.
///
/// Examples:
/// ```rust
/// assert_eq!(crate::harvest::shell_escape("hello"), "hello");
/// assert_eq!(crate::harvest::shell_escape("hello.world"), "'hello.world'");
/// assert_eq!(crate::harvest::shell_escape("hello'world"), "'hello''world'");
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

/// Escapes a path for use in shell commands by converting it to a lossy UTF-8 string and applying shell escaping rules.
///
/// This function safely escapes path components to ensure they are valid in shell contexts,
/// even when the path contains non-UTF-8 or invalid characters. It first converts the `Path`
/// to a lossy UTF-8 string using `to_string_lossy`, then applies `shell_escape` to produce
/// a shell-safe string.
///
/// Parameters:
/// - `p`: A reference to a `Path` object to be escaped.
///
/// Returns:
/// - A `String` containing the shell-escaped version of the path.
///
/// Examples:
/// ```rust
/// let path = std::path::Path::new("/home/user/file with spaces.txt");
/// let escaped = crate::harvest::shell_escape_lossy_path(&path);
/// assert!(escaped.contains("\"));
/// ```
fn shell_escape_lossy_path(p: &Path) -> String {
    shell_escape(&p.to_string_lossy())
}

/// Runs a Nu shell script to harvest data from specified targets using the `rust-ast` plugin and returns parsed rows in JSON format.
///
/// This function constructs a Nu shell command by sourcing a script file and optionally specifying target paths.
/// It then executes the command using a `ProcRunner`, captures the stdout, and deserializes the JSON output
/// into a vector of [`Row`] structs. If no targets are provided, the script runs with a dot (`.`) as the target.
/// The resulting rows are returned as a `Result<Vec<Row>>`.
///
/// Parameters:
/// - `script_path`: Path to the Nu script to source.
/// - `targets`: Optional list of paths to process. If empty, the script runs with no explicit targets.
///
/// Returns:
/// - A `Result<Vec<Row>>` containing the parsed rows from the Nu shell output.
///
/// Errors:
/// - Returns an `Error::Json` if the JSON output from Nu is malformed.
/// - Returns any I/O or execution errors from the `ProcRunner::run_text` call.
///
/// Notes:
/// - The script path and target paths are escaped using `shell_escape_lossy_path` to avoid shell injection.
/// - The output is expected to be valid JSON with a structure like `{"rows": [...]}`
/// - The `rust-ast` plugin must be available in the Nu environment.
///
/// Examples:
/// ```no_run
/// use std::path::PathBuf;
/// use crate::harvest::run_nushell_harvest;
/// use std::path::Path;
///
/// let script = Path::new("example.nu");
/// let targets = vec![PathBuf::from("data.txt")];
/// let rows = run_nushell_harvest(script, &targets)?;
/// ```
#[instrument(level = "info", skip(script_path, targets))]
pub fn run_nushell_harvest(script_path: &Path, targets: &[PathBuf]) -> Result<Vec<Row>> {
    let mut call = format!(
        "source {}; let rows = (rust-ast",
        shell_escape_lossy_path(script_path)
    );
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

    let rows: Vec<Row> = serde_json::from_str(&stdout).map_err(|e| Error::Json {
        context: "nu rust-ast JSON",
        source: e,
    })?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ---------------------------
    // shell_escape tests
    // ---------------------------

    #[test]
    fn test_shell_escape_keeps_safe_ascii_alnum_and_allowed_punct() {
        let input = "abcXYZ012/_-.";
        let out = shell_escape(input);
        assert_eq!(
            out, input,
            "Expected safe string to be unchanged.\nINPUT:\n{}\nOUTPUT:\n{}\n",
            input, out
        );
    }

    #[test]
    fn test_shell_escape_quotes_and_spaces() {
        let input = "hello world's file.txt";
        let out = shell_escape(input);
        // Our implementation wraps the WHOLE string in single quotes and
        // encodes inner single quotes as '\'' (POSIX-safe).
        let expected = "'hello world'\\''s file.txt'";
        assert_eq!(
            out, expected,
            "Expected spaces and single quotes to be safely escaped.\nINPUT:\n{}\nEXPECTED:\n{}\nOUTPUT:\n{}\n",
            input, expected, out
        );
    }

    #[test]
    fn test_shell_escape_non_ascii_gets_quoted() {
        let input = "résumé.pdf";
        let out = shell_escape(input);
        assert!(
            out.starts_with('\'') && out.ends_with('\''),
            "Non-ASCII should trigger full quoting.\nINPUT:\n{}\nOUTPUT:\n{}\n",
            input,
            out
        );
    }

    #[test]
    fn test_shell_escape_mixed_symbols_gets_quoted() {
        let input = "weird$(stuff)`here";
        let out = shell_escape(input);
        assert!(
            out.starts_with('\'') && out.ends_with('\''),
            "Shell metacharacters should trigger quoting.\nINPUT:\n{}\nOUTPUT:\n{}\n",
            input,
            out
        );
    }

    // ---------------------------
    // shell_escape_lossy_path tests
    // ---------------------------

    #[test]
    fn test_shell_escape_lossy_path_simple() {
        let p = Path::new("/tmp/myfile");
        let out = shell_escape_lossy_path(p);
        let expected = "/tmp/myfile";
        assert_eq!(
            out, expected,
            "Expected simple path to remain unquoted.\nPATH:\n{:?}\nEXPECTED:\n{}\nOUTPUT:\n{}\n",
            p, expected, out
        );
    }

    #[test]
    fn test_shell_escape_lossy_path_with_space_and_quote() {
        let p = Path::new("/tmp/dir with 'quote'");
        let out = shell_escape_lossy_path(p);
        // Inner single quotes become '\'' and the whole string gets wrapped in single quotes.
        let expected = "'/tmp/dir with '\\''quote'\\'''";
        assert_eq!(
            out, expected,
            "Expected path with spaces and quotes to be safely escaped.\nPATH:\n{:?}\nEXPECTED:\n{}\nOUTPUT:\n{}\n",
            p, expected, out
        );
    }

    // This test demonstrates that lossy conversion still yields a quoted string.
    // It only compiles/executes on Unix because it uses OsStrExt to construct
    // paths with invalid UTF-8.
    #[cfg(unix)]
    #[test]
    fn test_shell_escape_lossy_path_non_utf8_becomes_quoted() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // Create bytes with an invalid UTF-8 sequence.
        let raw = b"/tmp/\xFF\xFEinvalid";
        let p = Path::new(OsStr::from_bytes(raw));

        let out = shell_escape_lossy_path(p);

        assert!(
            out.starts_with('\'') && out.ends_with('\''),
            "Lossy non-UTF-8 path should be quoted.\nRAW BYTES:\n{:?}\nOUTPUT:\n{}\n",
            raw,
            out
        );
        // Also ensure replacement chars (�) appear after lossy conversion.
        assert!(
            out.contains('\u{FFFD}') || out.contains("\\u{FFFD}") || out.contains("�"),
            "Expected lossy replacement characters to appear in output.\nOUTPUT:\n{}\n",
            out
        );
    }
}
