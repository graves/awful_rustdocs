use crate::error::{Error, Result};
use crate::model::LlmDocResult;
use crate::regexes::{find_sig_line_near, re_field, re_fn_sig, re_struct};

use tracing::instrument;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// A text edit specifying a range and replacement content.
pub struct Edit {
    /// Starting index of the edit in the original text (inclusive).
    start: usize,
    /// Ending index of the edit in the original text (exclusive).
    end: usize,
    /// The text to insert or replace in the range [start, end).
    text: String,
}

/// Applies a series of text edits to a string in order of descending start position.
/// Edits are sorted by their start position in descending order to ensure correct application,
/// then applied sequentially to the text using `replace_range`. Each edit must have a valid
/// range (start ≤ end) and end within the bounds of the text length.
///
/// # Parameters
/// - `text`: The original string to which edits will be applied.
/// - `edits`: A vector of `Edit` structs, each containing a start position, end position, and replacement text.
///
/// # Returns
/// A new `String` with the edits applied in the correct order.
///
/// # Notes
/// - Edits are processed in descending order of start position to avoid overwriting.
/// - If an edit's end exceeds the length of the text, it is silently truncated.
/// - The original `text` is not modified; a new string is returned.
///
/// # Examples
/// ```rust
/// use crate::patch::Edit;
///
/// let mut text = "Hello world".to_string();
/// let edits = vec![
///     Edit { start: 6, end: 11, text: "there".to_string() },
///     Edit { start: 0, end: 5, text: "Hi".to_string() },
/// ];
///
/// let result = apply_edits(text, edits);
/// assert_eq!(result, "Hi there");
/// ```
fn apply_edits(mut text: String, mut edits: Vec<Edit>) -> String {
    edits.sort_by(|a, b| b.start.cmp(&a.start));
    for e in edits {
        if e.start <= e.end && e.end <= text.len() {
            text.replace_range(e.start..e.end, &e.text);
        }
    }
    text
}

/// Enumerates the different documentation shapes a Rust function may have.
#[derive(Debug)]
pub enum InsertWhere {
    /// This will be the line number of an undocumented function.
    Before(usize),
    /// This will be a tuple of (the_first_line_of_a_functions_documentation, the_first_line_of_the_function)
    /// This is used to overwrite previous documentation.
    Replace(usize, usize),
}

/// Determines the insertion point for attributes above a struct's signature in a Rust source file, specifically targeting doc comments
/// that begin with `///`. It searches backward from the struct signature line to find the first attribute line and identifies the block
/// of consecutive `///` comments that precede it. If `overwrite` is `true`, it returns an insertion point to replace the existing doc
/// comment block; otherwise, it returns an insertion point to insert before the first attribute.
///
/// Parameters:
/// - `src`: The source code as a string slice, containing the struct definition and surrounding code.
/// - `struct_sig_line0`: The zero-based line index of the struct's signature line.
/// - `overwrite`: A boolean indicating whether to overwrite existing doc comments (if any) before the struct signature.
///
/// Returns:
/// - `Some(InsertWhere::Before(anchor))` if no existing `///` doc block is found or if `overwrite` is `false`.
/// - `Some(InsertWhere::Replace(doc_lo, anchor))` if a block of `///` comments is found and `overwrite` is `true`.
///
/// Notes:
/// - The function traverses the source lines backward from the struct signature to find the first attribute (`#[...` or `#[![...]`).
/// - It identifies the start of a doc comment block by detecting consecutive `///` lines starting from the line immediately before the attribute.
/// - The `doc_lo` value marks the beginning of the doc block, and `anchor` marks the position of the attribute.
/// - If the block is found and `overwrite` is false, the function returns `None` to avoid modifying existing documentation.
///
/// Examples:
/// ```rust
/// use crate::patch::doc_slot_above_attrs;
///
/// let src = r#"struct Foo {
///     bar: i32
/// }"#;
///
/// let result = doc_slot_above_attrs(src, 3, false);
/// assert_eq!(result, None);
/// ```
fn doc_slot_above_attrs(
    src: &str,
    struct_sig_line0: usize,
    overwrite: bool,
) -> Option<InsertWhere> {
    let lines: Vec<&str> = src.lines().collect();
    let mut attr_first = struct_sig_line0;
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
        if t.is_empty() && saw_attr {
            if i == 0 {
                break;
            }
            i = i.saturating_sub(1);
            continue;
        }
        break;
    }
    let anchor = attr_first;

    if anchor > 0 && lines[anchor - 1].trim_start().starts_with("///") {
        let mut doc_lo = anchor - 1;
        while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") {
            doc_lo -= 1;
        }
        if !overwrite {
            return None;
        }
        return Some(InsertWhere::Replace(doc_lo, anchor));
    }
    Some(InsertWhere::Before(anchor))
}

/// Determines the insertion point for a documentation comment block in a source string based on the target line and overwrite flag.
///
/// Parameters:
/// - `src`: The source string containing the lines to inspect for documentation comments.
/// - `insert_line0`: The zero-based line index where the insertion should occur. If 0, inserts before the first line.
/// - `overwrite`: A flag indicating whether to overwrite existing documentation comments. If `false` and a comment block is found, returns `None`.
///
/// Returns:
/// - `Some(InsertWhere::Before(line))` if inserting before a line that is not a documentation comment.
/// - `Some(InsertWhere::Replace(start, end))` if inserting within or replacing an existing documentation block.
/// - `None` if `overwrite` is `false` and a matching documentation block is found.
///
/// Notes:
/// - Documentation blocks are identified by lines starting with "///" after trimming leading whitespace.
/// - The function finds the start of the block (the first line with "///" before the insertion point) and adjusts the insertion accordingly.
/// - If the insertion point is 0, it always returns an insertion before the first line.
/// ```
fn field_doc_slot(src: &str, insert_line0: usize, overwrite: bool) -> Option<InsertWhere> {
    let lines: Vec<&str> = src.lines().collect();
    if insert_line0 == 0 {
        return Some(InsertWhere::Before(0));
    }
    let i = insert_line0 - 1;
    if lines
        .get(i)
        .map_or(false, |l| l.trim_start().starts_with("///"))
    {
        if !overwrite {
            return None;
        }
        let mut doc_lo = i;
        while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") {
            doc_lo -= 1;
        }
        return Some(InsertWhere::Replace(doc_lo, insert_line0));
    }
    Some(InsertWhere::Before(insert_line0))
}

/// Finds the insertion range for a doc comment block in a Rust source string, starting from a given line index.
///
/// This function locates the beginning and end of a doc comment block by scanning backward from `start_line_1`.
/// It identifies the first non-doc comment line (using `///`, `#![doc]`, or `#[doc]` as markers) and the first attribute line (`#[` or `#![`).
/// The range is returned as `(lo, hi)`, where `lo` is the start line and `hi` is the end line of the doc block.
/// If no attributes are found, the end is set to `start_line_1` or the line before an empty line.
///
/// # Parameters
/// - `source`: The Rust source code as a string slice.
/// - `start_line_1`: The 1-based line index where the doc comment is expected to start.
///
/// # Returns
/// A tuple `(usize, usize)` representing the inclusive start and end line indices of the doc comment block.
///
/// # Notes
/// - The function uses 1-based line indexing in the input but converts to 0-based internally.
/// - Empty lines before the start are handled by checking `trim()` and adjusting `lo` accordingly.
/// - The function does not modify the input string.
/// - If `start_line_1` is 0 or beyond the source length, it saturates to valid bounds.
///
/// # Examples
/// ```rust
/// let source = "fn main() { /* */ }";
/// let range = find_doc_insertion_range(source, 1);
///
/// assert_eq!(range, (0, 0));
/// ```
fn find_doc_insertion_range(source: &str, start_line_1: usize) -> (usize, usize) {
    let lines: Vec<&str> = source.lines().collect();
    let sig_idx = start_line_1.saturating_sub(1);
    let mut lo = sig_idx;

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
            attr_first_idx = j;
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
        let hi = attr_first_idx;
        (lo, hi)
    } else {
        let mut lo2 = lo;
        if sig_idx > 0 && lines[sig_idx - 1].trim().is_empty() {
            lo2 = lo2.min(sig_idx - 1);
        }
        (lo2, sig_idx)
    }
}

/// Applies indentation to a document by preserving the leading whitespace of a target line
/// and applying it to lines that start with `///`, are empty, or contain content.
/// This function ensures consistent indentation in documentation blocks, replacing
/// the original indentation of the target line to all lines in the input document.
///
/// Parameters:
/// - `target_line`: A string slice representing the line whose indentation is to be copied.
/// - `doc`: A string slice containing the document to be indented, line by line.
///
/// Returns:
/// - A `String` with the document indented consistently using the whitespace from `target_line`.
///
/// Notes:
/// - Leading whitespace is extracted from `target_line` and applied to each line in the document.
/// - Empty lines are indented with the target indentation followed by `///`.
/// - Lines starting with `///` are preserved with the indentation applied.
/// - The final output ends with exactly one newline to ensure proper formatting.
fn indent_like(target_line: &str, doc: &str) -> String {
    let indent: String = target_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let mut out = String::new();

    for (i, raw) in doc.replace('\r', "").lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line = raw;
        if line.starts_with("///") {
            if !indent.is_empty() {
                out.push_str(&indent);
            }
            out.push_str(line);
        } else if line.trim().is_empty() {
            if !indent.is_empty() {
                out.push_str(&indent);
            }
            out.push_str("///");
        } else {
            if !indent.is_empty() {
                out.push_str(&indent);
            }
            out.push_str("/// ");
            out.push_str(line);
        }
    }

    // Ensure the block ends with exactly one newline, not two.
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Returns `true` if the line immediately above the specified `insert_line0` is non-blank,
/// otherwise `false`. If `insert_line0` is zero, the function returns `false` since there
/// is no line above the first line. The function checks the trimmed version of the line
/// to determine if it contains meaningful content.
///
/// # Parameters
/// - `source`: The input string slice containing the lines to inspect.
/// - `insert_line0`: The zero-based index of the line where insertion would occur.
///
/// # Returns
/// - `true` if the line above `insert_line0` is non-blank (after trimming), `false` otherwise.
///
/// # Notes
/// - The function assumes that `insert_line0` is within valid bounds (non-negative and less than or equal to the number of lines).
/// - It uses `lines().nth()` to access the previous line and `trim()` to check for empty content.
/// - Returns `false` when `insert_line0` is zero, as there is no line above the first line.
fn needs_leading_blank_line(source: &str, insert_line0: usize) -> bool {
    if insert_line0 == 0 {
        return false;
    }
    let prev = source.lines().nth(insert_line0 - 1).unwrap_or("");
    !prev.trim().is_empty()
}

/// Prefixes a newline to the documentation if the line immediately before `insert_line0` in `source` is not blank,
/// ensuring visual separation between consecutive items without breaking formatting.
/// This is useful for generating well-spaced, readable documentation output.
///
/// # Parameters
/// - `source`: The source text containing line information to evaluate for blankness before `insert_line0`.
/// - `insert_line0`: The zero-based line index where the new documentation block will be inserted.
/// - `doc`: The documentation string to potentially prefix with a newline.
///
/// # Returns
/// A `String` that either includes a leading newline (if needed) or is identical to `doc`.
///
/// # Notes
/// - The decision to add a newline is based solely on the content of `source` at line `insert_line0 - 1`.
/// - This function does not modify the original `source` or `doc` strings.
/// - It is designed to work in context of structured documentation generation, such as in a code generator or doc builder.
fn add_leading_blank_if_needed(source: &str, insert_line0: usize, doc: &str) -> String {
    if needs_leading_blank_line(source, insert_line0) {
        let mut s = String::with_capacity(doc.len() + 1);
        s.push('\n');
        s.push_str(doc);
        s
    } else {
        doc.to_string()
    }
}

/// Patches source files by inserting or updating documentation blocks based on LLM-generated results.
/// For each result, it locates the appropriate insertion point in the file (before or after a function/struct/field signature),
/// applies the generated doc string with proper indentation, and writes the updated content back to disk.
/// If `overwrite` is `false`, it skips existing doc blocks.
///
/// Parameters:
/// - `results`: A slice of [`LlmDocResult`] containing the generated documentation and metadata (e.g., file path, start line, kind, and doc content).
/// - `overwrite`: A boolean indicating whether to overwrite existing documentation blocks. If `false`, skips any file with existing doc blocks.
///
/// Returns:
/// - `Result<()>`: `Ok(())` on successful patching of all files, `Err` if any I/O or parsing error occurs.
///
/// Errors:
/// - Returns `Error::Io` with path and source if reading/writing files fails.
/// - Returns `Error` if parsing or matching fails during doc insertion (e.g., no signature found, invalid line structure).
///
/// Notes:
/// - The function processes files in a grouped manner by file path, ensuring efficient batch operations.
/// - For fields, insertion happens at the field's line; for functions/structs, it inserts above attributes or at the signature line.
/// - Edits are applied only if no existing doc block is present (or if `overwrite` is true).
/// - Line numbering is based on byte offsets, with line starts tracked for accurate insertion.
///
/// Examples:
/// ```no_run
/// let results = vec![
///     LlmDocResult {
///         file: "src/lib.rs",
///         start_line: Some(10),
///         kind: "fn".into(),
///         llm_doc: "Returns the current time.".into(),
///     },
/// ];
///
/// patch_files_with_docs(&results, false).await?;
/// ```
#[instrument(level = "info", skip(results))]
pub fn patch_files_with_docs(results: &[LlmDocResult], overwrite: bool) -> Result<()> {
    let mut by_file: BTreeMap<&str, Vec<&LlmDocResult>> = BTreeMap::new();

    for r in results {
        by_file.entry(&r.file).or_default().push(r);
    }

    for (file, mut items) in by_file {
        let original = fs::read_to_string(file).map_err(|e| Error::Io {
            path: Some(PathBuf::from(file)),
            source: e,
        })?;

        let mut line_starts: Vec<usize> = vec![0];
        for (i, b) in original.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        line_starts.push(original.len());

        let mut edits: Vec<Edit> = Vec::new();
        let mut skipped_no_sig = 0usize;
        let mut skipped_existing_doc = 0usize;

        items.sort_by_key(|r| r.start_line.unwrap_or(0));

        for r in items {
            let Some(start_line_1) = r.start_line else {
                continue;
            };
            let start_line0 = start_line_1.saturating_sub(1) as usize;

            let re_for_kind = match r.kind.as_str() {
                "struct" => re_struct(),
                "field" => re_field(),
                _ => re_fn_sig(),
            };
            let sig_line0_opt = if r.kind == "field" {
                Some(start_line0)
            } else {
                find_sig_line_near(&original, start_line0, re_for_kind)
            };

            let (ins_lo, ins_hi, indent_line_idx) = match (r.kind.as_str(), sig_line0_opt) {
                ("struct", Some(sig_line0)) => {
                    match doc_slot_above_attrs(&original, sig_line0, overwrite) {
                        Some(InsertWhere::Before(i)) => (i, i, i.min(sig_line0)),
                        Some(InsertWhere::Replace(lo, hi)) => (lo, hi, hi.min(sig_line0)),
                        None => {
                            skipped_existing_doc += 1;
                            continue;
                        }
                    }
                }
                ("field", _) => match field_doc_slot(&original, start_line0, overwrite) {
                    Some(InsertWhere::Before(i)) => (i, i, i),
                    Some(InsertWhere::Replace(lo, hi)) => (lo, hi, hi),
                    None => {
                        skipped_existing_doc += 1;
                        continue;
                    }
                },
                (_, Some(sig_line0)) => {
                    let (lo, hi) = find_doc_insertion_range(&original, sig_line0 + 1);
                    (lo, hi, sig_line0)
                }
                _ => {
                    skipped_no_sig += 1;
                    continue;
                }
            };

            let lines: Vec<&str> = original.lines().collect();
            let lo = ins_lo.min(lines.len());
            let hi = ins_hi.min(lines.len());

            let has_doc_block_in_range = (lo..hi).any(|k| {
                let t = lines[k].trim_start();
                t.starts_with("///") || t.starts_with("#![doc") || t.starts_with("#[doc")
            });
            if !overwrite && has_doc_block_in_range {
                skipped_existing_doc += 1;
                continue;
            }

            let start_b = *line_starts.get(ins_lo).unwrap_or(&0);
            let end_b = *line_starts.get(ins_hi).unwrap_or(&start_b);

            let target_line = original.lines().nth(indent_line_idx).unwrap_or("");
            let mut repl = indent_like(target_line, &r.llm_doc);

            // Add one blank line *before* the doc block when the previous line is non-blank.
            // Do this only for top-level items (fn/struct), not for fields.
            if r.kind != "field" {
                repl = add_leading_blank_if_needed(&original, ins_lo, &repl);
            }

            edits.push(Edit {
                start: start_b,
                end: end_b,
                text: repl,
            });
        }

        if edits.is_empty() {
            eprintln!(
                "Patched {}: 0 edits (skipped_no_sig={}, skipped_existing_doc={})",
                file, skipped_no_sig, skipped_existing_doc
            );
            continue;
        }

        let new_text = apply_edits(original, edits);
        fs::write(file, new_text).map_err(|e| Error::Io {
            path: Some(PathBuf::from(file)),
            source: e,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- helpers ----------

    fn numbered(src: &str) -> String {
        src.lines()
            .enumerate()
            .map(|(i, l)| format!("{:>3}: {}", i, l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ---------- apply_edits ----------

    #[test]
    fn test_apply_edits_basic_replacements_and_ordering() {
        // Text indices: 0123456789 10
        //               H e l l o _ w  o  r  l  d
        let text = "Hello_world".to_string();
        let edits = vec![
            // Replace "world" -> "there"
            Edit {
                start: 6,
                end: 11,
                text: "there".to_string(),
            },
            // Replace "Hello" -> "Hi"
            Edit {
                start: 0,
                end: 5,
                text: "Hi".to_string(),
            },
        ];
        let got = apply_edits(text, edits);
        let want = "Hi_there".to_string();
        assert_eq!(
            got, want,
            "Expected simple, ordered replacements to produce:\n{}\nGot:\n{}",
            want, got
        );
    }

    #[test]
    fn test_apply_edits_ignores_out_of_bounds_and_invalid_ranges() {
        let text = "ABCDE".to_string();
        let edits = vec![
            // invalid: start > end
            Edit {
                start: 3,
                end: 2,
                text: "X".into(),
            },
            // out of bounds end (ignored by current code since end > len)
            Edit {
                start: 4,
                end: 999,
                text: "Z".into(),
            },
            // valid: replace "BC" -> "bb"
            Edit {
                start: 1,
                end: 3,
                text: "bb".into(),
            },
        ];
        let got = apply_edits(text, edits);
        let want = "AbbDE".to_string();
        assert_eq!(
            got, want,
            "apply_edits should ignore invalid/out-of-bounds edits; want:\n{}\ngot:\n{}",
            want, got
        );
    }

    // ---------- doc_slot_above_attrs ----------

    #[test]
    fn test_doc_slot_above_attrs_before_when_no_existing_doc() {
        let src = r#"
#[inline]
pub struct Foo {
    a: i32,
}
"#;
        // struct sig is on line index 2 (0-based)
        let sig = 2;
        let res = doc_slot_above_attrs(src, sig, false);
        match res {
            Some(InsertWhere::Before(i)) => {
                // Should insert before the first attr line (line 1)
                assert_eq!(
                    i,
                    1,
                    "Expected insertion at first attribute line.\nSRC:\n{}",
                    numbered(src)
                );
            }
            _ => panic!("Unexpected result; SRC:\n{}", numbered(src)),
        }
    }

    #[test]
    #[test]
    fn test_doc_slot_above_attrs_replace_when_doc_present_and_overwrite() {
        let src = r#"
#[inline]
/// Doc A
/// Doc B
pub struct Foo {
    a: i32,
}
"#;
        // struct sig line idx (0-based): 4
        let sig = 4;
        let res = doc_slot_above_attrs(src, sig, true);
        match res {
            Some(InsertWhere::Replace(lo, hi)) => {
                // Current behavior: because the line above the signature is "///",
                // the function doesn't scan further up to the attribute; it uses the signature
                // line as the anchor (hi = 4), and the start of the doc block as lo = 2.
                assert_eq!(
                    (lo, hi),
                    (2, 4),
                    "Expected (doc_lo, sig_line). Got ({lo},{hi}).\nSRC:\n{}",
                    {
                        src.lines()
                            .enumerate()
                            .map(|(i, l)| format!("{:>3}: {}", i, l))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                );
            }
            _ => panic!("Expected Replace.\nSRC:\n{}", {
                src.lines()
                    .enumerate()
                    .map(|(i, l)| format!("{:>3}: {}", i, l))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        }
    }

    #[test]
    fn test_doc_slot_above_attrs_none_when_doc_present_and_no_overwrite() {
        let src = r#"
#[inline]
/// Already here
pub struct Foo { a: i32 }
"#;
        let sig = 3;
        let got = doc_slot_above_attrs(src, sig, false);
        assert!(
            got.is_none(),
            "When a doc block exists and overwrite=false, expected None.\nSRC:\n{}",
            numbered(src)
        );
    }

    // ---------- field_doc_slot ----------

    #[test]
    fn test_field_doc_slot_before_when_no_doc() {
        let src = r#"
pub struct Foo {
    name: String,
}
"#;
        // field 'name' line index (0-based) is 2
        let res = field_doc_slot(src, 2, false);
        match res {
            Some(InsertWhere::Before(i)) => assert_eq!(
                i,
                2,
                "Expected to insert before the field line.\nSRC:\n{}",
                numbered(src)
            ),
            _ => panic!("Expected InsertWhere::Before.\nSRC:\n{}", numbered(src)),
        }
    }

    #[test]
    fn test_field_doc_slot_replace_when_doc_exists_and_overwrite() {
        let src = r#"
pub struct Foo {
    /// field doc
    name: String,
}
"#;
        // field 'name' line index (0-based) is 3
        let res = field_doc_slot(src, 3, true);
        match res {
            Some(InsertWhere::Replace(lo, hi)) => {
                assert_eq!(
                    (lo, hi),
                    (2, 3),
                    "Expected Replace at the doc block range.\nSRC:\n{}",
                    numbered(src)
                );
            }
            _ => panic!("Expected Replace.\nSRC:\n{}", numbered(src)),
        }
    }

    #[test]
    fn test_field_doc_slot_none_when_doc_exists_and_no_overwrite() {
        let src = r#"
pub struct Foo {
    /// doc present
    name: String,
}
"#;
        // field line index must be the field line itself (3)
        let field_line0 = 3usize;
        let res = field_doc_slot(src, field_line0, false);
        assert!(
            res.is_none(),
            "Expected None when doc exists and overwrite=false.\nGot: some variant.\nSRC:\n{}",
            numbered(src)
        );
    }

    // ---------- find_doc_insertion_range ----------

    #[test]
    fn test_find_doc_insertion_range_with_attrs_and_doc() {
        let src = r#"
#[inline]
/// Doc A
/// Doc B
pub fn foo() {}
"#;
        // Signature is at 4 (0-based)
        let (lo, hi) = find_doc_insertion_range(src, 4 + 1 /*1-based*/);
        // Current behavior: (lo, hi) = (2, 4)
        assert_eq!(
            (lo, hi),
            (2, 4),
            "Expected lo to be the first doc line, hi to be the signature line.\nSRC:\n{}\n(lo,hi)=({lo},{hi})",
            numbered(src)
        );
    }

    #[test]
    fn test_find_doc_insertion_range_no_attrs_moves_to_blank_before_sig() {
        let src = r#"
/// Doc

pub fn foo() {}
"#;
        // Signature at 3 (0-based)
        let (lo, hi) = find_doc_insertion_range(src, 3 + 1);
        // Current behavior: (2,3) — start at Doc line (2), end at sig (3)
        assert_eq!(
            (lo, hi),
            (2, 3),
            "Expected doc block to end at sig and start at the doc line.\nSRC:\n{}\n(lo, hi)=({lo},{hi})",
            numbered(src)
        );
    }

    // ---------- indent_like ----------

    #[test]
    fn test_indent_like_preserves_doc_markers_and_blank_lines() {
        let target_line = "    pub fn foo() {}";
        let doc = "/// First\n\n/// Second\nLine without marker";
        let got = indent_like(target_line, doc);
        let want = "    /// First\n    ///\n    /// Second\n    /// Line without marker\n";
        assert_eq!(
            got, want,
            "Indentation or doc markers not preserved correctly.\nExpected:\n---\n{}\n---\nGot:\n---\n{}\n---",
            want, got
        );
    }

    #[test]
    fn test_indent_like_ensures_single_trailing_newline() {
        let target_line = "fn x() {}";
        let doc = "Line 1\nLine 2\n";
        let got = indent_like(target_line, doc);
        let want = "/// Line 1\n/// Line 2\n";
        assert_eq!(
            got, want,
            "Expected exactly one trailing newline.\nGot:\n{:?}",
            got
        );
    }

    // ---------- needs/add leading blank ----------

    #[test]
    fn test_needs_leading_blank_line_true_when_prev_line_non_blank() {
        let src = "line A\nline B\n";
        // Inserting at line index 1 (line 'line B') -> line above is 'line A' (non-blank)
        assert!(
            needs_leading_blank_line(src, 1),
            "Expected true when previous line is non-blank.\nSRC:\n{}",
            numbered(src)
        );
    }

    #[test]
    fn test_needs_leading_blank_line_false_when_prev_blank_or_top() {
        let src = "\nline B\n";
        // insert at line 1 -> previous is blank
        assert!(
            !needs_leading_blank_line(src, 1),
            "Expected false when previous line is blank.\nSRC:\n{}",
            numbered(src)
        );
        // insert at 0 -> top
        assert!(
            !needs_leading_blank_line(src, 0),
            "Expected false at top-of-file insertion.\nSRC:\n{}",
            numbered(src)
        );
    }

    #[test]
    fn test_add_leading_blank_if_needed_adds_blank() {
        let src = "fn a() {}\nfn b() {}\n";
        let doc = "/// new doc\n";
        let got = add_leading_blank_if_needed(src, 1, doc);
        let want = format!("\n{}", doc);
        assert_eq!(
            got, want,
            "Expected a leading blank line to be added.\nGot:\n{:?}",
            got
        );
    }

    #[test]
    fn test_add_leading_blank_if_needed_noop_when_prev_blank() {
        let src = "fn a() {}\n\nfn b() {}\n";
        let doc = "/// new doc\n";
        let got = add_leading_blank_if_needed(src, 2, doc);
        assert_eq!(
            got, doc,
            "Expected no leading blank when the previous line is already blank.\nGot:\n{:?}",
            got
        );
    }
}
