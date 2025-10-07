use regex::Regex;
use serde::{Deserialize, Serialize};

use std::collections::BTreeSet;

/// A span representing a range of lines and bytes in a text document.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Span {
    /// Start line of the span, 1-indexed. `None` if not set.
    pub start_line: Option<u32>,
    /// End line of the span, 1-indexed. `None` if not set.
    pub end_line: Option<u32>,
    /// Start byte offset of the span, 0-indexed. `None` if not set.
    pub start_byte: Option<u64>,
    /// End byte offset of the span, 0-indexed. `None` if not set.
    pub end_byte: Option<u64>,
}

/// A row representing a single item in the codebase (e.g., function, struct, enum). Contains metadata about its definition, visibility, and context.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Row {
    /// The type of the row (e.g., "function", "struct", "enum").
    pub kind: String,
    /// The name of the item (e.g., function name, struct name).
    pub name: String,
    /// The crate name, with alternate aliases (crate, crate_, crate_field). May be None.
    #[serde(rename = "crate", alias = "crate_", alias = "crate_field")]
    pub crate_name: Option<String>,
    /// Optional list of module path segments. May be None.
    pub module_path: Option<Vec<String>>,
    /// Fully qualified path (e.g., `crate::module::item`).
    pub fqpath: String,
    /// Visibility of the item (e.g., pub, private).
    pub visibility: String,
    /// Source file path where the item is defined.
    pub file: String,
    /// Source span (location) of the item.
    pub span: Span,
    /// Function or method signature as a string.
    pub signature: String,
    /// Whether the item has a body (e.g., function or struct with implementation).
    pub has_body: bool,
    /// Optional documentation string.
    pub doc: Option<String>,
    /// Optional body text content (e.g., implementation block).
    pub body_text: Option<String>,
    /// Optional list of caller names (e.g., functions calling this item).
    pub callers: Option<Vec<String>>,
}

impl Row {
    /// Checks whether the associated document has non-empty content after trimming whitespace.
    ///
    /// Returns `true` if the document field is present and contains non-empty text after trimming; otherwise `false`.
    ///
    /// # Returns
    /// - `bool`: `true` if the document has non-empty content, `false` otherwise.
    ///
    /// # Notes
    /// - Uses `as_deref()` to safely unwrap the optional document string.
    /// - Trims whitespace and checks if the result is non-empty using `is_empty()`.
    /// - This function does not modify the internal state of the struct.
    ///
    /// # Examples
    /// ```rust
    /// use crate::model::Row;
    ///
    /// let row = Row { doc: Some("  hello world  ".to_string()) };
    /// assert!(row.had_doc());
    ///
    /// let row = Row { doc: Some("   ".to_string()) };
    /// assert!(!row.had_doc());
    ///
    /// let row = Row { doc: None };
    /// assert!(!row.had_doc());
    /// ```
    pub fn had_doc(&self) -> bool {
        self.doc.as_deref().map_or(false, |d| !d.trim().is_empty())
    }

    /// Returns the start and end byte positions of the span, with default values of 0 and `u64::MAX` if the span boundaries are missing.
    ///
    /// Parameters: None
    ///
    /// Returns: A tuple of two `u64` values: the start byte position and the end byte position.
    /// - The start byte is `0` if `self.span.start_byte` is `None`.
    /// - The end byte is `u64::MAX` if `self.span.end_byte` is `None`.
    ///
    /// Errors: None
    ///
    /// Notes: This function safely handles missing span boundaries by using `unwrap_or` with default values. It does not panic even if the span fields are absent.
    ///
    /// Examples:
    /// ```rust
    /// use crate::model::Row;
    ///
    /// let row = Row { span: None };
    /// assert_eq!(row.span_bytes(), (0, u64::MAX));
    /// ```
    pub fn span_bytes(&self) -> (u64, u64) {
        (
            self.span.start_byte.unwrap_or(0),
            self.span.end_byte.unwrap_or(u64::MAX),
        )
    }
}

/// Result of LLM-generated documentation for a code item, containing metadata and generated content.
#[derive(Debug, Serialize, Clone)]
pub struct LlmDocResult {
    /// The kind of documentation (e.g., "function", "type", "struct").
    pub kind: String,
    /// The fully qualified path to the item in the codebase (e.g., "crate::module::item").
    pub fqpath: String,
    /// The filename where the item is defined.
    pub file: String,
    /// The starting line number of the item in the source file (optional).
    pub start_line: Option<u32>,
    /// The ending line number of the item in the source file (optional).
    pub end_line: Option<u32>,
    /// The function or item signature (e.g., "fn foo(x: i32) -> u32").
    pub signature: String,
    /// List of calling functions that reference this item.
    pub callers: Vec<String>,
    /// List of symbols referenced by this item (e.g., variables, functions).
    pub referenced_symbols: Vec<String>,
    /// The generated documentation content from LLM (e.g., markdown or text).
    pub llm_doc: String,
    /// Whether the item already had existing documentation before generation.
    pub had_existing_doc: bool,
}

/// Enum field documentation strings
#[derive(Debug, Deserialize)]
pub struct FieldDocOut {
    /// Name of the field in the enum.
    pub name: String,
    /// Field-level documentation returned from the LLM.
    pub doc: String,
}

/// Response containing structured documentation of a model.
/// Includes a string representation of the model doc and a list of field-level documentation entries.
#[derive(Debug, Deserialize)]
pub struct StructDocResponse {
    /// The structured documentation of the model.
    pub struct_doc: String,
    /// List of field documentation entries.
    /// Each entry describes a field in the model structure.
    pub fields: Vec<FieldDocOut>,
}

/// Finds all function references that mention a given struct name or fully-qualified struct name in their body text.
///
/// Parameters:
/// - `struct_name`: The name of the struct to search for. Matches using word boundaries.
/// - `struct_fq`: The fully-qualified name of the struct to search for.
/// - `fns`: A slice of references to function rows, each containing a `body_text` and `fqpath`.
///
/// Returns:
/// A sorted, deduplicated list of fully-qualified paths (`String`) of functions that contain either `struct_name` or `struct_fq` in their body text.
///
/// Errors:
/// - None. The function uses `unwrap` on `Regex::new`, which may panic on invalid regex patterns, but this is not exposed as a specific error.
///
/// Notes:
/// - Uses `regex::escape` to safely escape struct names for regex matching.
/// - Matches using word boundaries (`\b`) to avoid partial matches.
/// - Returns only functions whose body text contains the struct name or fully-qualified name.
///
/// Examples:
/// ```rust
/// use crate::model::Row;
///
/// let fns = vec![
///     Row { body_text: Some("This function references MyStruct".into()), fqpath: "mod1::func1".into() },
///     Row { body_text: Some("Calls MyStruct::new()".into()), fqpath: "mod2::func2".into() },
///     Row { body_text: Some("No reference here".into()), fqpath: "mod3::func3".into() },
/// ];
///
/// let result = referencing_functions("MyStruct", "mod1::MyStruct", &fns);
/// assert_eq!(result, vec!["mod1::func1", "mod2::func2"]);
/// ```
pub fn referencing_functions(struct_name: &str, struct_fq: &str, fns: &[&Row]) -> Vec<String> {
    let word_name = Regex::new(&format!(r"\b{}\b", regex::escape(struct_name))).unwrap();
    let word_fq = Regex::new(&regex::escape(struct_fq)).unwrap();

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

/// Collects symbol references from a given text body using a regex pattern and a set of known symbols.
///
/// This function scans the input `body` for matches against the provided `word_re` regex pattern.
/// For each match, it checks if the matched word is present in `all_symbols`. If so, it adds the
/// word (as a string) to a `BTreeSet` of found symbols, limiting the collection to at most 64 symbols.
/// The result is returned as a sorted vector of unique symbol references.
///
/// Parameters:
/// - `body`: The input text to search for symbol references.
/// - `all_symbols`: A set of known symbol names to match against.
/// - `word_re`: A regex pattern used to find word matches in the body.
///
/// Returns:
/// - A sorted vector of strings containing the found symbol references, up to 64 entries.
///
/// Notes:
/// - The function stops early if 64 symbols are found to prevent excessive processing.
/// - Matches are case-sensitive and must exactly match a word boundary.
/// - Empty input returns an empty vector.
///
/// Examples:
/// ```rust
/// use regex::Regex;
/// use std::collections::BTreeSet;
///
/// let word_re = Regex::new(r"\b\w+\b").unwrap();
/// let all_symbols = BTreeSet::new();
/// let body = "hello world hello";
/// let result = collect_symbol_refs(body, &all_symbols, &word_re);
///
/// assert_eq!(result, vec![]);
/// ```
pub fn collect_symbol_refs(
    body: &str,
    all_symbols: &BTreeSet<String>,
    word_re: &Regex,
) -> Vec<String> {
    if body.is_empty() {
        return vec![];
    }
    let mut found = BTreeSet::new();
    for m in word_re.find_iter(body) {
        let w = m.as_str();
        if all_symbols.contains(w) {
            found.insert(w.to_string());
            if found.len() == 64 {
                break;
            }
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use serde_json::json;
    use std::collections::BTreeSet;

    // ---------- helpers ----------

    fn mk_span(
        start_line: Option<u32>,
        end_line: Option<u32>,
        start_byte: Option<u64>,
        end_byte: Option<u64>,
    ) -> Span {
        Span {
            start_line,
            end_line,
            start_byte,
            end_byte,
        }
    }

    fn mk_row_with(kind: &str, name: &str, fqpath: &str, body_text: Option<&str>) -> Row {
        Row {
            kind: kind.to_string(),
            name: name.to_string(),
            crate_name: None,
            module_path: None,
            fqpath: fqpath.to_string(),
            visibility: "pub".to_string(),
            file: "src/lib.rs".to_string(),
            span: mk_span(Some(1), Some(1), Some(0), Some(0)),
            signature: format!("{kind} {name}()"),
            has_body: true,
            doc: None,
            body_text: body_text.map(str::to_string),
            callers: None,
        }
    }

    // ---------- Row::had_doc ----------

    #[test]
    fn test_row_had_doc_true_when_non_empty_after_trim() {
        let row = Row {
            kind: "fn".into(),
            name: "foo".into(),
            crate_name: None,
            module_path: None,
            fqpath: "crate::foo".into(),
            visibility: "pub".into(),
            file: "src/lib.rs".into(),
            span: mk_span(None, None, None, None),
            signature: "fn foo()".into(),
            has_body: true,
            doc: Some("  hello  ".into()),
            body_text: None,
            callers: None,
        };
        assert!(
            row.had_doc(),
            "Expected had_doc() to be true for doc = {:?}",
            row.doc
        );
    }

    #[test]
    fn test_row_had_doc_false_on_whitespace_or_none() {
        let mut row = mk_row_with("fn", "foo", "crate::foo", None);
        row.doc = Some("   \n\t".into());
        assert!(
            !row.had_doc(),
            "Expected had_doc() to be false for whitespace-only doc: {:?}",
            row.doc
        );
        row.doc = None;
        assert!(
            !row.had_doc(),
            "Expected had_doc() to be false when doc is None"
        );
    }

    // ---------- Row::span_bytes ----------

    #[test]
    fn test_row_span_bytes_defaults_when_missing() {
        let row = Row {
            span: mk_span(None, None, None, None),
            ..mk_row_with("fn", "foo", "crate::foo", None)
        };
        let (start, end) = row.span_bytes();
        assert_eq!(
            (start, end),
            (0, u64::MAX),
            "Expected span_bytes() to default to (0, u64::MAX); got ({start}, {end})"
        );
    }

    #[test]
    fn test_row_span_bytes_uses_present_values() {
        let row = Row {
            span: mk_span(None, None, Some(10), Some(99)),
            ..mk_row_with("fn", "foo", "crate::foo", None)
        };
        let (start, end) = row.span_bytes();
        assert_eq!(
            (start, end),
            (10, 99),
            "Expected span_bytes() to return provided values; got ({start}, {end})"
        );
    }

    // ---------- serde aliasing for crate_name ----------

    #[test]
    fn test_crate_name_deserializes_from_crate() {
        let j = json!({
            "kind":"fn","name":"foo","crate":"my_crate",
            "module_path":["a","b"],"fqpath":"a::b::foo","visibility":"pub","file":"src/lib.rs",
            "span":{"start_line":1,"end_line":1,"start_byte":0,"end_byte":0},
            "signature":"fn foo()","has_body":true
        });
        let row: Row = serde_json::from_value(j).expect("deserialize Row");
        assert_eq!(
            row.crate_name.as_deref(),
            Some("my_crate"),
            "Expected crate_name to read from `crate` key"
        );
    }

    #[test]
    fn test_crate_name_deserializes_from_crate_underscore() {
        let j = json!({
            "kind":"fn","name":"foo","crate_":"alt_crate",
            "module_path":null,"fqpath":"foo","visibility":"pub","file":"src/lib.rs",
            "span":{"start_line":1,"end_line":1,"start_byte":0,"end_byte":0},
            "signature":"fn foo()","has_body":true
        });
        let row: Row = serde_json::from_value(j).expect("deserialize Row");
        assert_eq!(
            row.crate_name.as_deref(),
            Some("alt_crate"),
            "Expected crate_name to read from `crate_` key"
        );
    }

    #[test]
    fn test_crate_name_deserializes_from_crate_field() {
        let j = json!({
            "kind":"fn","name":"foo","crate_field":"field_crate",
            "module_path":null,"fqpath":"foo","visibility":"pub","file":"src/lib.rs",
            "span":{"start_line":1,"end_line":1,"start_byte":0,"end_byte":0},
            "signature":"fn foo()","has_body":true
        });
        let row: Row = serde_json::from_value(j).expect("deserialize Row");
        assert_eq!(
            row.crate_name.as_deref(),
            Some("field_crate"),
            "Expected crate_name to read from `crate_field` key"
        );
    }

    // ---------- referencing_functions ----------

    #[test]
    fn test_referencing_functions_matches_name_and_fqpath() {
        // Functions referencing either `MyStruct` or `mod1::MyStruct`
        let f1 = mk_row_with("fn", "f1", "mod1::f1", Some("let _ = MyStruct::new();"));
        let f2 = mk_row_with("fn", "f2", "mod2::f2", Some("uses mod1::MyStruct in text"));
        let f3 = mk_row_with("fn", "f3", "mod3::f3", Some("no reference here"));
        let all = vec![f1, f2, f3];
        let refs: Vec<&Row> = all.iter().collect();

        let out = referencing_functions("MyStruct", "mod1::MyStruct", &refs);
        assert_eq!(
            out,
            vec!["mod1::f1", "mod2::f2"],
            "Expected functions referencing either name or fqpath.\nFOUND:\n{:#?}",
            out
        );
    }

    #[test]
    fn test_referencing_functions_uses_word_boundaries() {
        // Should not match `MyStructX`
        let f1 = mk_row_with("fn", "f1", "m::f1", Some("MyStructX should not match"));
        let f2 = mk_row_with("fn", "f2", "m::f2", Some("wrap MyStruct here"));
        let refs: Vec<&Row> = vec![&f1, &f2];

        let out = referencing_functions("MyStruct", "m::MyStruct", &refs);
        assert_eq!(
            out,
            vec!["m::f2"],
            "Expected only exact word-boundary matches.\nFOUND:\n{:#?}",
            out
        );
    }

    // ---------- collect_symbol_refs ----------

    #[test]
    fn test_collect_symbol_refs_empty_body() {
        let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();
        let set: BTreeSet<String> = ["Foo", "Bar"].iter().map(|s| s.to_string()).collect();
        let out = collect_symbol_refs("", &set, &re);
        assert!(
            out.is_empty(),
            "Expected no symbols for empty body; got: {out:#?}"
        );
    }

    #[test]
    fn test_collect_symbol_refs_finds_known_symbols() {
        let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();
        let all: BTreeSet<String> = ["Foo", "Bar", "baz"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // No comments; only Foo and Bar appear.
        let body = "fn go() { let x = Foo::new(); Bar::zap(x); }";
        let out = collect_symbol_refs(body, &all, &re);
        assert_eq!(
            out,
            vec!["Bar".to_string(), "Foo".to_string()],
            "Expected only known symbols in lexical order.\nBODY:\n{}\nOUTPUT:\n{:#?}",
            body,
            out
        );
    }

    #[test]
    fn test_collect_symbol_refs_limits_to_64() {
        let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();

        // Build set of 100 known symbols S0..S99
        let mut all = BTreeSet::new();
        for i in 0..100 {
            all.insert(format!("S{i}"));
        }
        // Body includes all 100 once
        let mut body = String::new();
        for i in 0..100 {
            body.push_str(&format!(" S{i}()"));
        }

        let out = collect_symbol_refs(&body, &all, &re);
        assert!(
            out.len() <= 64,
            "Expected collection to cap at 64 symbols; got {}\nOUTPUT(first 10): {:#?}",
            out.len(),
            &out[..out.len().min(10)]
        );
    }
}
