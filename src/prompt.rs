use crate::grep::CallSite;
use crate::model::Row;

/// Truncates a string to fit within a specified number of characters and lines, preserving line breaks and adding a truncation indicator if necessary.
///
/// Parameters:
/// - `s`: The input string to truncate.
/// - `max_chars`: The maximum number of characters allowed in the output.
/// - `max_lines`: The maximum number of lines allowed in the output.
///
/// Returns:
/// A truncated string that does not exceed `max_chars` and contains at most `max_lines` lines. If the length exceeds `max_chars`, it is truncated and ends with `// …truncated…`.
///
/// Errors:
/// None. The function always returns a valid string.
///
/// Notes:
/// - The function preserves line breaks and truncates the content based on the number of lines first, then on character count.
/// - If the total length after joining lines exceeds `max_chars`, the string is truncated and a comment is appended.
/// - This is useful for displaying long texts in contexts with limited space, such as UI elements or logs.
pub fn truncate_for_context(s: &str, max_chars: usize, max_lines: usize) -> String {
    let mut out = s.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    if out.len() > max_chars {
        out.truncate(max_chars);
        out.push_str("\n// …truncated…");
    }
    out
}

/// Builds a markdown-formatted question from a function's metadata, including its path, signature, and referenced symbols.
///
/// This function constructs a structured markdown representation of a function's context, useful for documentation or introspection.
/// It includes details such as the fully-qualified path, signature, visibility, referenced symbols, and function calls within its span.
/// The output is a formatted string suitable for rendering in documentation systems or developer tools.
///
/// Parameters:
/// - `f`: A reference to a `Row` containing function metadata (path, signature, visibility, etc.).
/// - `referenced_symbols`: A slice of symbol names referenced within the function body.
/// - `calls_in_span`: A slice of `CallSite` entries representing function calls within the span.
///
/// Returns:
/// - A `String` containing the formatted markdown question.
///
/// Notes:
/// - The function truncates the function body to 400 characters for display, using a context limit of 8000.
/// - Only the first 50 calls in the span are included to avoid excessive output.
/// - If no existing documentation is present, it will indicate "_No existing rustdoc found._"
///
/// Examples:
/// ```no_run
/// let row = Row {
///     fqpath: "crate::example::function",
///     signature: "fn hello() -> String",
///     visibility: "pub",
///     doc: None,
///     body_text: Some("return format!("Hello, world!");"),
///     referenced_symbols: &["format!"],
///     calls_in_span: &[CallSite { kind: "call", callee: "format!", qual: Some("format!") }],
/// };
///
/// let question = build_markdown_question(&row, &["format!"], &[CallSite { kind: "call", callee: "format!", qual: Some("format!") }]);
///
/// println!("{}", question);
/// ```
pub fn build_markdown_question(
    f: &Row,
    referenced_symbols: &[String],
    calls_in_span: &[CallSite],
) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    writeln!(s, "# Rust Function Documentation Task").ok();
    writeln!(s, "You are given context about a single Rust function.").ok();
    writeln!(s).ok();

    writeln!(s, "## Function Identity").ok();
    writeln!(s, "- **Fully-qualified path**: `{}`", f.fqpath).ok();
    writeln!(s, "- **Signature**: `{}`", f.signature).ok();
    writeln!(s, "- **Visibility**: `{}`", f.visibility).ok();

    writeln!(s, "\n## Existing Documentation").ok();
    match &f.doc {
        Some(doc) if !doc.trim().is_empty() => {
            writeln!(
                s,
                "The function already has Rustdoc. Improve and rewrite it if necessary:"
            )
            .ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => {
            writeln!(s, "_No existing rustdoc found._").ok();
        }
    };

    writeln!(s, "\n## Referenced Symbols (body-level)").ok();
    if referenced_symbols.is_empty() {
        writeln!(s, "_No symbol references detected._").ok();
    } else {
        for sym in referenced_symbols {
            writeln!(s, "- `{}`", sym).ok();
        }
    }

    if !calls_in_span.is_empty() {
        writeln!(s, "\n## Function Calls Inside This Function").ok();
        for c in calls_in_span.iter().take(50) {
            match &c.qual {
                Some(q) => {
                    writeln!(s, "- **{}** call → `{}` on `{}`", c.kind, c.callee, q).ok();
                }
                None => {
                    writeln!(s, "- **{}** call → `{}`", c.kind, c.callee).ok();
                }
            };
        }
    }

    if let Some(body) = &f.body_text {
        writeln!(s, "\n## Function Body (Truncated)").ok();
        let trimmed = truncate_for_context(body, 8000, 400);
        writeln!(s, "```rust\n{}\n```", trimmed).ok();
    }

    writeln!(s, "\n---\n## Output Requirements\n\
        Return **ONLY** a Rustdoc block composed of lines starting with `///`.\n\
        - No JSON, no backticks, no XML, no surrounding prose.\n\
        - Include a clear 1–2 sentence summary.\n\
        - If relevant, add sections titled exactly: `Parameters:`, `Returns:`, `Errors:`, `Notes:`, `Examples:`.\n\
        - Only include a `Safety:` section if the function is unsafe.
        - Use concise bullet points; examples should be doc-test friendly (no fenced code).\n\
        - Every line MUST start with `///` (or be a blank `///`)."
    ).ok();

    s
}

/// Builds a structured request string for generating Rustdoc for a given struct, including its metadata, existing documentation, body, and referencing functions.
///
/// The function constructs a detailed prompt that includes the struct's fully-qualified path, signature, visibility, existing Rustdoc (if any), struct body (verbatim), and up to 100 referencing function paths. It then specifies the expected output format: a JSON object with a `struct_doc` field (a concise 1–2 sentence summary) and a list of `fields`, each containing a `doc` entry for a named field in the struct body.
///
/// Parameters:
/// - `srow`: A reference to a `Row` containing struct metadata including fully-qualified path, signature, and visibility.
/// - `body_text`: The raw Rust struct body text as a string slice.
/// - `referencing_fns`: A slice of strings representing the fully-qualified paths of functions that reference this struct.
///
/// Returns:
/// - A `String` containing the formatted prompt ready to be used in a model or LLM for generating Rustdoc.
///
/// Notes:
/// - The function limits the number of referencing functions to 100 to prevent excessive prompt length.
/// - If no existing documentation is present, it explicitly notes "_No existing rustdoc found._".
/// - The output is structured to guide an AI model to produce valid, concise, and accurate Rustdoc comments.
///
/// Examples:
/// ```no_run
/// use crate::prompt::Row;
///
/// let srow = Row {
///     fqpath: "crate::my_struct".into(),
///     signature: "pub struct MyStruct { pub field: i32 }".into(),
///     visibility: "pub".into(),
///     doc: None,
///     body_text: "pub struct MyStruct { pub field: i32 }".into(),
/// };
///
/// let referencing_fns = &["crate::util::process", "crate::core::handle"];
/// let prompt = build_struct_request_with_refs(&srow, "pub struct MyStruct { pub field: i32 }", referencing_fns);
///
/// println!("{}", prompt);
/// ```
pub fn build_struct_request_with_refs(
    srow: &Row,
    body_text: &str,
    referencing_fns: &[String],
) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    writeln!(s, "# Rust Struct Documentation Task").ok();
    writeln!(s, "You are given the source of a single Rust struct and a list of functions that reference it.").ok();

    writeln!(s, "\n## Struct Identity").ok();
    writeln!(s, "- **Fully-qualified path**: `{}`", srow.fqpath).ok();
    writeln!(s, "- **Signature**: `{}`", srow.signature).ok();
    writeln!(s, "- **Visibility**: `{}`", srow.visibility).ok();

    writeln!(s, "\n## Existing Documentation").ok();
    match &srow.doc {
        Some(doc) if !doc.trim().is_empty() => {
            writeln!(
                s,
                "The struct already has Rustdoc. If needed, rewrite it to be concise:"
            )
            .ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => {
            writeln!(s, "_No existing rustdoc found._").ok();
        }
    };

    writeln!(s, "\n## Struct Body (verbatim)").ok();
    writeln!(s, "```rust\n{}\n```", body_text).ok();

    writeln!(s, "\n## Referencing Functions (FQ paths)").ok();
    if referencing_fns.is_empty() {
        writeln!(s, "_No referencing functions detected in the crate._").ok();
    } else {
        for f in referencing_fns.iter().take(100) {
            writeln!(s, "- `{}`", f).ok();
        }
    }

    writeln!(s, "\n---\n## Output Requirements").ok();
    writeln!(
        s,
        "Respond in **structured JSON** (no prose) with this shape:"
    )
    .ok();
    writeln!(
        s,
        r#"{{
  "struct_doc": "/// short summary...\n/// ...",
  "fields": [
    {{ "name": "field_name", "doc": "/// one-line or short doc...\n/// ..." }}
  ]
}}"#
    )
    .ok();
    writeln!(
        s,
        "- `struct_doc`: A short 1–2 sentence rustdoc for the struct (above attributes)."
    )
    .ok();
    writeln!(s, "- `fields`: One entry **per named field** appearing in the struct body; the `doc` value must be a ready-to-insert `///` block for that field (keep it short, include units/invariants if relevant).").ok();

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grep::CallSite;
    use crate::model::{Row, Span};

    // ---------- helpers ----------

    fn mk_span() -> Span {
        Span {
            start_line: Some(10),
            end_line: Some(20),
            start_byte: Some(100),
            end_byte: Some(200),
        }
    }

    fn mk_row_fn(doc: Option<&str>, body: Option<&str>) -> Row {
        Row {
            kind: "fn".into(),
            name: "hello".into(),
            crate_name: Some("crate_name".into()),
            module_path: Some(vec!["moda".into(), "modb".into()]),
            fqpath: "crate::moda::modb::hello".into(),
            visibility: "pub".into(),
            file: "src/lib.rs".into(),
            span: mk_span(),
            signature: "pub fn hello(x: i32) -> i32".into(),
            has_body: true,
            doc: doc.map(|s| s.to_string()),
            body_text: body.map(|s| s.to_string()),
            callers: Some(vec!["crate::main::run".into()]),
        }
    }

    fn mk_row_struct(doc: Option<&str>) -> Row {
        Row {
            kind: "struct".into(),
            name: "Widget".into(),
            crate_name: Some("crate_name".into()),
            module_path: Some(vec!["moda".into()]),
            fqpath: "crate::moda::Widget".into(),
            visibility: "pub".into(),
            file: "src/lib.rs".into(),
            span: mk_span(),
            signature: "pub struct Widget { pub w: usize }".into(),
            has_body: true,
            doc: doc.map(|s| s.to_string()),
            body_text: None,
            callers: None,
        }
    }

    // ---------- truncate_for_context ----------

    #[test]
    fn test_truncate_for_context_respects_line_limit() {
        let s = "a\nb\nc\nd\ne";
        let out = truncate_for_context(s, 10_000, 3);
        assert_eq!(out, "a\nb\nc", "FULL OUTPUT:\n{out}");
    }

    #[test]
    fn test_truncate_for_context_respects_char_limit_and_appends_marker() {
        let s = "0123456789abcdefghij"; // 20 chars
        let out = truncate_for_context(s, 12, 10);
        // Should truncate to 12 chars, then append newline + marker
        let expected = "0123456789ab\n// …truncated…";
        assert_eq!(out, expected, "FULL OUTPUT:\n{out}");
    }

    #[test]
    fn test_truncate_for_context_no_truncation_when_within_limits() {
        let s = "line1\nline2";
        let out = truncate_for_context(s, 100, 10);
        assert_eq!(out, s, "FULL OUTPUT:\n{out}");
    }

    // ---------- build_markdown_question (no existing doc) ----------

    #[test]
    fn test_build_markdown_question_no_existing_doc_includes_core_sections() {
        let row = mk_row_fn(None, Some("fn hello(){ let _x = 1; }"));
        let refs = vec!["Foo".to_string(), "Bar".to_string()];
        let calls = vec![
            CallSite {
                kind: "plain".into(),
                qual: None,
                callee: "zap".into(),
            },
            CallSite {
                kind: "qualified".into(),
                qual: Some("pkg::util".into()),
                callee: "fmt".into(),
            },
        ];

        let out = build_markdown_question(&row, &refs, &calls);

        // identity
        assert!(out.contains("## Function Identity"), "FULL OUTPUT:\n{out}");
        assert!(
            out.contains("`crate::moda::modb::hello`"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("`pub fn hello(x: i32) -> i32`"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("- **Visibility**: `pub`"),
            "FULL OUTPUT:\n{out}"
        );

        // existing doc section should say none
        assert!(
            out.contains("_No existing rustdoc found._"),
            "FULL OUTPUT:\n{out}"
        );

        // referenced symbols list
        assert!(
            out.contains("## Referenced Symbols (body-level)"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(out.contains("- `Foo`"), "FULL OUTPUT:\n{out}");
        assert!(out.contains("- `Bar`"), "FULL OUTPUT:\n{out}");

        // calls
        assert!(
            out.contains("## Function Calls Inside This Function"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("- **plain** call → `zap`"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("- **qualified** call → `fmt` on `pkg::util`"),
            "FULL OUTPUT:\n{out}"
        );

        // body block
        assert!(
            out.contains("## Function Body (Truncated)"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(out.contains("```rust"), "FULL OUTPUT:\n{out}");
        assert!(out.contains("let _x = 1;"), "FULL OUTPUT:\n{out}");

        // output requirements
        assert!(
            out.contains("## Output Requirements"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("Return **ONLY** a Rustdoc block"),
            "FULL OUTPUT:\n{out}"
        );
    }

    #[test]
    fn test_build_markdown_question_includes_only_first_50_calls() {
        let row = mk_row_fn(None, None);
        let refs: Vec<String> = vec![];
        // 60 calls -> should only list 50
        let calls: Vec<CallSite> = (0..60)
            .map(|i| CallSite {
                kind: "plain".into(),
                qual: None,
                callee: format!("f{i}"),
            })
            .collect();

        let out = build_markdown_question(&row, &refs, &calls);
        let count = out.matches("- **plain** call → `").count();
        assert_eq!(
            count, 50,
            "Expected exactly 50 calls to be rendered.\nFULL OUTPUT:\n{out}"
        );
        // sanity: first and last of the expected slice appear
        assert!(out.contains("`f0`"), "FULL OUTPUT:\n{out}");
        assert!(out.contains("`f49`"), "FULL OUTPUT:\n{out}");
        // and one beyond 49 should not appear
        assert!(!out.contains("`f50`"), "FULL OUTPUT:\n{out}");
    }

    #[test]
    fn test_build_markdown_question_with_existing_doc_embeds_code_block() {
        let row = mk_row_fn(Some("Existing doc\nMore lines"), Some("fn body() {}"));
        let out = build_markdown_question(&row, &[], &[]);
        // Should embed the trimmed doc in a rust code block
        assert!(
            out.contains("The function already has Rustdoc."),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("```rust\nExisting doc\nMore lines\n```"),
            "FULL OUTPUT:\n{out}"
        );
    }

    // ---------- build_struct_request_with_refs ----------

    #[test]
    fn test_build_struct_request_with_refs_no_existing_doc_and_no_refs() {
        let srow = mk_row_struct(None);
        let body = "pub struct Widget { pub w: usize }";
        let out = build_struct_request_with_refs(&srow, body, &[]);

        assert!(
            out.contains("# Rust Struct Documentation Task"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(out.contains("## Struct Identity"), "FULL OUTPUT:\n{out}");
        assert!(out.contains("`crate::moda::Widget`"), "FULL OUTPUT:\n{out}");
        assert!(
            out.contains("`pub struct Widget { pub w: usize }`"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("_No existing rustdoc found._"),
            "FULL OUTPUT:\n{out}"
        );

        // body verbatim
        assert!(
            out.contains("## Struct Body (verbatim)"),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("```rust\npub struct Widget { pub w: usize }\n```"),
            "FULL OUTPUT:\n{out}"
        );

        // refs
        assert!(
            out.contains("_No referencing functions detected in the crate._"),
            "FULL OUTPUT:\n{out}"
        );

        // output JSON shape guidance
        assert!(
            out.contains(r#""struct_doc": "/// short summary..."#),
            "FULL OUTPUT:\n{out}"
        );
        assert!(out.contains(r#""fields": ["#), "FULL OUTPUT:\n{out}");
    }

    #[test]
    fn test_build_struct_request_with_refs_limits_to_100_refs() {
        let srow = mk_row_struct(None);
        let body = "pub struct Widget { pub w: usize }";

        let all_refs: Vec<String> = (0..150).map(|i| format!("crate::f::{i}")).collect();

        let out = build_struct_request_with_refs(&srow, body, &all_refs);

        // Count how many "- `...`" lines for refs appear; expect 100
        let rendered = out
            .lines()
            .filter(|l| l.trim_start().starts_with("- `crate::f::"))
            .count();
        assert_eq!(
            rendered, 100,
            "Expected 100 refs to be listed.\nFULL OUTPUT:\n{out}"
        );

        // Sanity: first and boundary values
        assert!(out.contains("- `crate::f::0`"), "FULL OUTPUT:\n{out}");
        assert!(out.contains("- `crate::f::99`"), "FULL OUTPUT:\n{out}");
        assert!(!out.contains("- `crate::f::100`"), "FULL OUTPUT:\n{out}");
    }

    #[test]
    fn test_build_struct_request_with_refs_shows_existing_doc_when_present() {
        let srow = mk_row_struct(Some("Existing struct doc.\nMore."));
        let body = "pub struct Widget { pub w: usize }";
        let out = build_struct_request_with_refs(&srow, body, &[]);
        assert!(
            out.contains("The struct already has Rustdoc."),
            "FULL OUTPUT:\n{out}"
        );
        assert!(
            out.contains("```rust\nExisting struct doc.\nMore.\n```"),
            "FULL OUTPUT:\n{out}"
        );
    }
}
