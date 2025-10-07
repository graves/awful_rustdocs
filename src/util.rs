use crate::regexes::{re_attr, re_field};
use regex::Regex;

/// Describes a field specification extracted from a source file.
/// Contains metadata about the field's location, name, and context.
#[derive(Debug)]
pub struct FieldSpec {
    /// The name of the field as it appears in the source code.
    /// Must be a valid identifier and unique within its parent struct.
    pub name: String,
    /// The line number in the source file where the field is first declared.
    /// Line numbers are 1-based and refer to the file's source text.
    pub field_line0: usize,
    /// The line number in the source file where the field insertion point is specified.
    /// Line numbers are 1-based and refer to the file's source text.
    pub insert_line0: usize,
    /// The fully qualified path to the parent struct or type.
    /// Used for navigation and context in field resolution.
    pub parent_fqpath: String,
    /// The raw text of the field line as it appears in the source file.
    /// Includes the field declaration syntax and any modifiers.
    pub field_line_text: String,
}

/// Extracts a range of lines from a string based on zero-based line indices.
///
/// This function takes a string slice and two zero-based line indices (`lo_line0` and `hi_line0`)
/// and returns a string containing only the lines in that range, joined by newline characters.
/// The range is inclusive of both endpoints. If the range is invalid (e.g., `lo_line0 > hi_line0`),
/// the function will return an empty string.
///
/// # Parameters
/// - `src`: The input string slice to extract lines from.
/// - `lo_line0`: The starting line index (inclusive), zero-based.
/// - `hi_line0`: The ending line index (inclusive), zero-based.
///
/// # Returns
/// - A `String` containing the lines between `lo_line0` and `hi_line0`, inclusive, joined by newlines. If the range is invalid or out of bounds, returns an empty string.
pub fn extract_lines(src: &str, lo_line0: usize, hi_line0: usize) -> String {
    src.lines()
        .enumerate()
        .filter(|(i, _)| *i >= lo_line0 && *i <= hi_line0)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Finds the start and end line indices of a struct's body block in source code, starting from a given line index.
///
/// This function scans the source code line by line, beginning at `struct_sig_line0`, to locate the opening `{` and then tracks
/// brace nesting until it finds the matching closing `}` that balances the opening brace. It returns an `Option<(usize, usize)>`
/// containing the start and end line indices of the struct body block. If no valid block is found, it returns `None`.
///
/// # Parameters
/// - `src`: The source code as a string slice.
/// - `struct_sig_line0`: The line index where the struct signature begins (e.g., `struct MyStruct`).
///
/// # Returns
/// - `Some((start, end))`: The start and end line indices of the struct body block.
/// - `None`: If no matching block is found or the source code is malformed.
///
/// # Notes
/// - The function assumes that struct bodies are enclosed in `{}` and that braces are properly nested.
/// - It does not handle comments or other syntax that might interfere with brace matching.
/// - The line indices are 0-based and refer to the line number in the input string.
pub fn find_struct_body_block(src: &str, struct_sig_line0: usize) -> Option<(usize, usize)> {
    let mut brace_line_start = None;
    let mut open = 0i32;
    for (i, line) in src.lines().enumerate().skip(struct_sig_line0) {
        if brace_line_start.is_none() {
            if let Some(_pos) = line.find('{') {
                brace_line_start = Some((i, 0));
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

/// Extracts field specifications from a Rust struct's body in a source code string, identifying fields defined with attributes and their positions.
/// The function parses the source code between `body_start_line0` and `body_end_line0`, detecting lines that match field patterns using regex,
/// and constructs `FieldSpec` entries for each valid field. It respects attribute boundaries and tracks the field's line number, insertion point, and parent file path.
///
/// # Parameters
/// - `file_src`: The source code string containing the struct definition to parse.
/// - `body_start_line0`: The zero-based line number where the struct body starts (after the opening `{`).
/// - `body_end_line0`: The zero-based line number where the struct body ends (inclusive).
/// - `parent_fqpath`: The full qualified path of the parent file, used to enrich the field metadata.
///
/// # Returns
/// A `Vec<FieldSpec>` containing all detected field definitions with their line numbers, names, and insertion points.
///
/// # Notes
/// - Field detection uses regex patterns to match valid Rust field declarations, ignoring `pub` and `r#` prefixes.
/// - The function skips lines that do not match attribute or field patterns.
/// - The `insert_line0` is set to the top of the attribute block or the field line, whichever is earlier.
/// - Empty field names are filtered out.
///
/// # Examples
/// ```rust
/// use crate::util::FieldSpec;
///
/// let src = r#"struct Example { pub name: String; pub age: u32; }"#;
/// let fields = extract_struct_fields_in_file(src, 2, 5, "src/main.rs");
///
/// assert_eq!(fields.len(), 2);
/// assert_eq!(fields[0].name, "name");
/// assert_eq!(fields[1].name, "age");
/// ```
pub fn extract_struct_fields_in_file(
    file_src: &str,
    body_start_line0: usize,
    body_end_line0: usize,
    parent_fqpath: &str,
) -> Vec<FieldSpec> {
    let lines: Vec<&str> = file_src.lines().collect();
    let mut out = Vec::new();

    let mut i = body_start_line0 + 1; // after the '{'
    while i < lines.len() && i <= body_end_line0.saturating_sub(1) {
        let mut j = i;
        let attr_top = j;
        while j <= body_end_line0 && j < lines.len() && re_attr().is_match(lines[j].trim_start()) {
            j += 1;
        }
        if j <= body_end_line0 && j < lines.len() {
            let l = lines[j];
            if re_field().is_match(l) {
                let name = Regex::new(
                    r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\s*:"#,
                )
                .unwrap()
                .captures(l)
                .and_then(|c| c.get(1))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a string with 0-based line numbers for easier human reading in failures.
    fn with_line_numbers(s: &str) -> String {
        s.lines()
            .enumerate()
            .map(|(i, line)| format!("{:>3}: {}", i, line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Construct a minimal Rust source containing a single `pub struct Example { ... }`.
    /// Returns the full source and the (start, end) 0-based line indices for the body block.
    fn make_struct_src(body_lines: &[&str]) -> (String, (usize, usize)) {
        let mut src = String::new();
        src.push_str("mod m {}\n"); // 0
        src.push_str("\n"); // 1
        src.push_str("pub struct Example \n"); // 2
        src.push_str("{\n"); // 3 (opening brace on its own line)
        for l in body_lines {
            src.push_str(l);
            src.push('\n');
        }
        src.push_str("}\n"); // last

        let sig_line0 = 2; // "pub struct Example"
        let body = find_struct_body_block(&src, sig_line0).expect("body block not found");
        (src, body)
    }

    #[test]
    fn test_extract_lines_basic_ranges() {
        let src = "a\nb\nc\nd\ne";
        let got = extract_lines(src, 0, 0);
        let expect = "a";
        assert_eq!(
            got,
            expect,
            "single line (0..=0)\nGOT:\n{}\nEXPECT:\n{}",
            with_line_numbers(&got),
            with_line_numbers(expect)
        );

        let got = extract_lines(src, 1, 3);
        let expect = "b\nc\nd";
        assert_eq!(
            got,
            expect,
            "middle range (1..=3)\nGOT:\n{}\nEXPECT:\n{}",
            with_line_numbers(&got),
            with_line_numbers(expect)
        );

        let got = extract_lines(src, 4, 4);
        let expect = "e";
        assert_eq!(
            got,
            expect,
            "last line (4..=4)\nGOT:\n{}\nEXPECT:\n{}",
            with_line_numbers(&got),
            with_line_numbers(expect)
        );

        let got = extract_lines(src, 3, 1);
        let expect = "";
        assert_eq!(
            got,
            expect,
            "invalid range (3..=1) should be empty\nGOT:\n{}\nEXPECT:\n{}",
            with_line_numbers(&got),
            with_line_numbers(expect)
        );

        let got = extract_lines(src, 2, 99);
        let expect = "c\nd\ne";
        assert_eq!(
            got,
            expect,
            "hi beyond bounds should clamp\nGOT:\n{}\nEXPECT:\n{}",
            with_line_numbers(&got),
            with_line_numbers(expect)
        );
    }

    #[test]
    fn test_find_struct_body_block_simple() {
        let (src, (lo, hi)) = make_struct_src(&["x: i32,", "y: String,"]);
        let lines: Vec<_> = src.lines().collect();

        assert_eq!(
            lines[lo].trim(),
            "{",
            "body must start at the line containing the opening brace\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
        assert_eq!(
            lines[hi].trim(),
            "}",
            "body must end at the line containing the closing brace\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );

        // There are exactly two field lines between the braces.
        assert_eq!(hi - lo - 1, 2, "expected 2 body lines between braces");
    }

    #[test]
    fn test_extract_struct_fields_simple_two_fields() {
        let (src, (lo, hi)) = make_struct_src(&["name: String,", "age: u32,"]);
        let fields = extract_struct_fields_in_file(&src, lo, hi, "crate::Example");
        assert_eq!(
            fields.len(),
            2,
            "expected two fields\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );

        // Field 0
        assert_eq!(fields[0].name, "name", "first field should be `name`");
        assert!(
            fields[0].field_line_text.contains("name: String"),
            "field line text should contain the declaration; got:\n{}",
            fields[0].field_line_text
        );
        assert_eq!(
            fields[0].insert_line0, fields[0].field_line0,
            "with no attributes, insert_line0 should equal field_line0"
        );

        // Field 1
        assert_eq!(fields[1].name, "age", "second field should be `age`");
        assert!(
            fields[1].field_line_text.contains("age: u32"),
            "field line text should contain the declaration; got:\n{}",
            fields[1].field_line_text
        );
        assert_eq!(
            fields[1].insert_line0, fields[1].field_line0,
            "with no attributes, insert_line0 should equal field_line0"
        );

        // Parent path
        for f in &fields {
            assert_eq!(
                f.parent_fqpath, "crate::Example",
                "parent_fqpath should be threaded through"
            );
        }
    }

    #[test]
    fn test_extract_struct_fields_with_attributes_groups_to_attr_top() {
        let (src, (lo, hi)) = make_struct_src(&[
            r#"#[serde(rename = "n")]"#,
            "pub name: String,",
            r#"#[doc = "age in years"]"#,
            "pub age: u32,",
        ]);
        let fields = extract_struct_fields_in_file(&src, lo, hi, "crate::Example");
        assert_eq!(
            fields.len(),
            2,
            "expected two fields with attributes\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );

        let lines: Vec<&str> = src.lines().collect();

        let attr_name_top = lines
            .iter()
            .position(|l| l.trim() == r#"#[serde(rename = "n")]"#)
            .expect("missing name attribute");
        let name_line = lines
            .iter()
            .position(|l| l.trim() == "pub name: String,")
            .expect("missing name line");

        let attr_age_top = lines
            .iter()
            .position(|l| l.trim() == r#"#[doc = "age in years"]"#)
            .expect("missing age attribute");
        let age_line = lines
            .iter()
            .position(|l| l.trim() == "pub age: u32,")
            .expect("missing age line");

        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[0].field_line0, name_line);
        assert_eq!(
            fields[0].insert_line0, attr_name_top,
            "insert_line0 should be at the top of the attribute block for `name`"
        );

        assert_eq!(fields[1].name, "age");
        assert_eq!(fields[1].field_line0, age_line);
        assert_eq!(
            fields[1].insert_line0, attr_age_top,
            "insert_line0 should be at the top of the attribute block for `age`"
        );
    }

    #[test]
    fn test_extract_struct_fields_handles_pub_and_raw_identifiers() {
        let (src, (lo, hi)) = make_struct_src(&[
            "pub(crate) id: u64,",
            "r#type: String,",
            "pub r#match: bool,",
        ]);
        let fields = extract_struct_fields_in_file(&src, lo, hi, "crate::Example");
        let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["id", "type", "match"],
            "should normalize identifiers and ignore visibility/raw prefixes\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }

    #[test]
    fn test_extract_struct_fields_respects_body_bounds() {
        // Build a struct with two fields; then append a bogus "field" after the closing brace.
        let (mut src, (lo, hi)) = make_struct_src(&["a: i32,", "b: i32,"]);
        src.push_str("c: i32,\n"); // outside the struct body

        let fields = extract_struct_fields_in_file(&src, lo, hi, "crate::Example");
        let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["a", "b"],
            "must ignore lines after the struct body\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
        assert!(
            !names.contains(&"c"),
            "`c` should be ignored as it's outside the body"
        );
    }

    #[test]
    fn test_find_struct_body_block_nested_braces_in_body_lines() {
        // Ensure our simple brace counter remains robust when braces appear in lines,
        // e.g., in comments or string literals.
        let (src, (lo, hi)) = make_struct_src(&[
            "a: i32,",
            r#"// pretend { nested } braces in a comment"#,
            r#"b: std::borrow::Cow<'static, str>,"#,
        ]);
        let lines: Vec<_> = src.lines().collect();

        assert_eq!(
            lines[lo].trim(),
            "{",
            "body must start at opening brace\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
        assert_eq!(
            lines[hi].trim(),
            "}",
            "body must end at closing brace\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
        assert_eq!(
            hi - lo - 1,
            3,
            "expected exactly three lines in the body\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }
}
