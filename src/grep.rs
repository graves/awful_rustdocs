use crate::error::Result;
use crate::runner::ToolRunner;

use serde::Deserialize;
use tracing::instrument;

use std::collections::BTreeSet;

/// A record representing a snippet of source code with its file path, range, and optional text content.
#[derive(Debug, Deserialize)]
pub struct SgRecord {
    /// The file path associated with this record.
    pub file: String,
    /// The source range within the file, represented as a range.
    #[serde(rename = "range")]
    pub range: SgRange,
    /// Optional text content extracted from the file range.
    pub text: Option<String>,
    /// Metadata variables associated with the record, initialized to default.
    #[serde(default)]
    pub metaVariables: SgMetaVars,
}

/// A byte range representation used in memory or data offsets.
#[derive(Debug, Deserialize)]
pub struct SgRange {
    /// Byte offset range, serialized as "byteOffset" in JSON.
    #[serde(rename = "byteOffset")]
    pub byte: SgByteRange,
}

/// A byte range defined by start and end offsets.
#[derive(Debug, Deserialize)]
pub struct SgByteRange {
    /// Starting byte index (inclusive).
    pub start: u64,
    /// Ending byte index (exclusive).
    pub end: u64,
}

/// Metadata variables stored as JSON values. Used to hold dynamic, serializable metadata in a flexible format.
#[derive(Debug, Default, Deserialize)]
pub struct SgMetaVars {
    /// The serialized JSON value representing a single metadata variable.
    #[serde(default)]
    pub single: serde_json::Value,
}

/// Represents a call site in code, capturing its kind, optional qualifier, and callee name.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// The type of call site: "plain", "qualified", or "method".
    pub kind: String,
    /// Optional qualifier for qualified calls (e.g., module or path prefix).
    pub qual: Option<String>,
    /// The name of the function or method being called.
    pub callee: String,
}

/// Extracts SG records from a file within a specified byte span using `ast-grep` and a given pattern.
///
/// Parameters:
/// - `runner`: A dynamic reference to a `ToolRunner` used to execute the `ast-grep` tool.
/// - `file`: The path to the file being searched for records.
/// - `pattern`: The pattern to match against the file's source code.
/// - `start`: The starting byte offset (inclusive) within the file to include in the search.
/// - `end`: The ending byte offset (exclusive) within the file to include in the search.
///
/// Returns:
/// - A `Result<Vec<SgRecord>>` containing the SG records that match the pattern and fall within the specified byte range.
///
/// Errors:
/// - Returns `crate::error::Error::Json` if a JSON parsing error occurs while deserializing a line from `ast-grep`.
/// - Returns any error from `runner.run_json_lines` if the tool execution fails.
///
/// Notes:
/// - The function uses `ast-grep` with `--json=stream` to stream JSON-serialized records.
/// - Only records whose range byte boundaries fall strictly within `[start, end]` are included.
/// - The input `start` and `end` must be valid byte offsets within the file.
fn records_in_span(
    runner: &dyn ToolRunner,
    file: &str,
    pattern: &str,
    start: u64,
    end: u64,
) -> Result<Vec<SgRecord>> {
    let lines = runner.run_json_lines(
        "ast-grep",
        &[
            "run",
            "-l",
            "rust",
            "-p",
            pattern,
            "--json=stream",
            "--heading=never",
            "--color=never",
            file,
        ],
    )?;
    let mut out = Vec::new();
    for line in lines {
        let rec: SgRecord = serde_json::from_str(&line).map_err(|e| crate::error::Error::Json {
            context: "ast-grep line",
            source: e,
        })?;
        if rec.range.byte.start >= start && rec.range.byte.end <= end {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Extracts call sites within a specified byte span of a source file using pattern matching on meta-variables.
/// For each pattern (`"$N($$$A)"`, `"$Q::$N($$$A)"`, `"$RECV.$N($$$A)"`), it queries the tool runner to find matching records in the given file range, then parses the `N`, `Q`, and `RECV` fields from the meta-variables to construct `CallSite` entries. The resulting list of call sites is returned, including the call kind, qualified name (if applicable), and the callee name.
///
/// # Parameters:
/// - `runner`: A dynamic reference to a [`ToolRunner`] that provides access to tool-based analysis and metadata extraction.
/// - `file`: The path or name of the source file being analyzed.
/// - `start_byte`: The starting byte offset (inclusive) within the file to search for call sites.
/// - `end_byte`: The ending byte offset (exclusive) within the file to search for call sites.
///
/// # Returns:
/// A `Result<Vec<CallSite>>` containing the list of detected call sites within the specified span, or an error if any step fails.
///
/// # Errors:
/// - Returns an error if `records_in_span` fails to retrieve records in the specified span.
/// - Any I/O or parsing errors from the `ToolRunner` during meta-variable extraction are propagated.
///
/// # Notes:
/// - The function supports three call pattern types: plain, qualified, and method, each with distinct parsing logic.
/// - Empty `name` values are filtered out to avoid invalid call site entries.
/// - The `qual` field is only populated for qualified and method calls.
#[instrument(level = "debug", skip(runner))]
pub fn calls_in_function_span(
    runner: &dyn ToolRunner,
    file: &str,
    start_byte: u64,
    end_byte: u64,
) -> Result<Vec<CallSite>> {
    let mut out = Vec::new();
    for (pat, kind) in [
        ("$N($$$A)", "plain"),
        ("$Q::$N($$$A)", "qualified"),
        ("$RECV.$N($$$A)", "method"),
    ] {
        let recs = records_in_span(runner, file, pat, start_byte, end_byte)?;
        for r in recs {
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
    Ok(out)
}

/// Extracts qualified path names from a file span using pattern matching on records within a byte range.
///
/// This function queries a `ToolRunner` to retrieve records in a specified byte range of a file,
/// matching against predefined pattern strings. For each record, it extracts the text, trims whitespace,
/// and inserts into a `BTreeSet` if the text contains a `::` delimiter, indicating a qualified path.
/// The result is a sorted set of unique qualified path strings.
///
/// Parameters:
/// - `runner`: A dynamic reference to a `ToolRunner` that can query records in a file.
/// - `file`: The filename or path to query records from.
/// - `start_byte`: The starting byte position (inclusive) of the span to query.
/// - `end_byte`: The ending byte position (exclusive) of the span to query.
///
/// Returns:
/// - A `Result` containing a `BTreeSet<String>` of qualified path strings, or an error if the query fails.
///
/// Errors:
/// - Returns errors from `records_in_span` if the query fails.
/// - Errors from I/O or parsing during record retrieval are bubbled up.
///
/// Notes:
/// - The patterns `$Q::$N`, `$Q::<$$$A>::$N`, and `$Q::{$$$A}` are used to match qualified paths.
/// - Only paths containing `::` are included in the output.
/// - The result is guaranteed to be sorted due to the use of `BTreeSet`.
pub fn qualified_paths_in_span(
    runner: &dyn ToolRunner,
    file: &str,
    start_byte: u64,
    end_byte: u64,
) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for pat in ["$Q::$N", "$Q::<$$$A>::$N", "$Q::{$$$A}"] {
        let recs = records_in_span(runner, file, pat, start_byte, end_byte)?;
        for r in recs {
            if let Some(txt) = r.text.as_ref() {
                let t = txt.trim();
                if t.contains("::") {
                    paths.insert(t.to_string());
                }
            }
        }
    }
    Ok(paths)
}
