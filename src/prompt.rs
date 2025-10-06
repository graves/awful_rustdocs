use crate::model::Row;
use crate::grep::CallSite;

pub fn truncate_for_context(s: &str, max_chars: usize, max_lines: usize) -> String {
    let mut out = s.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    if out.len() > max_chars {
        out.truncate(max_chars);
        out.push_str("\n// …truncated…");
    }
    out
}

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
            writeln!(s, "The function already has Rustdoc. Improve and rewrite it if necessary:").ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => { writeln!(s, "_No existing rustdoc found._").ok(); }
    };

    writeln!(s, "\n## Referenced Symbols (body-level)").ok();
    if referenced_symbols.is_empty() {
        writeln!(s, "_No symbol references detected._").ok();
    } else {
        for sym in referenced_symbols { writeln!(s, "- `{}`", sym).ok(); }
    }

    if !calls_in_span.is_empty() {
        writeln!(s, "\n## Function Calls Inside This Function").ok();
        for c in calls_in_span.iter().take(50) {
            match &c.qual {
                Some(q) => { writeln!(s, "- **{}** call → `{}` on `{}`", c.kind, c.callee, q).ok(); }
                None    => { writeln!(s, "- **{}** call → `{}`", c.kind, c.callee).ok(); }
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
        - If relevant, add sections titled exactly: `Parameters:`, `Returns:`, `Errors:`, `Safety:`, `Notes:`, `Examples:`.\n\
        - Use concise bullet points; examples should be doc-test friendly (no fenced code).\n\
        - Every line MUST start with `///` (or be a blank `///`)."
    ).ok();

    s
}

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
            writeln!(s, "The struct already has Rustdoc. If needed, rewrite it to be concise:").ok();
            writeln!(s, "```rust\n{}\n```", doc.trim()).ok();
        }
        _ => { writeln!(s, "_No existing rustdoc found._").ok(); }
    };

    writeln!(s, "\n## Struct Body (verbatim)").ok();
    writeln!(s, "```rust\n{}\n```", body_text).ok();

    writeln!(s, "\n## Referencing Functions (FQ paths)").ok();
    if referencing_fns.is_empty() {
        writeln!(s, "_No referencing functions detected in the crate._").ok();
    } else {
        for f in referencing_fns.iter().take(100) { writeln!(s, "- `{}`", f).ok(); }
    }

    writeln!(s, "\n---\n## Output Requirements").ok();
    writeln!(s, "Respond in **structured JSON** (no prose) with this shape:").ok();
    writeln!(s, r#"{{
  "struct_doc": "/// short summary...\n/// ...",
  "fields": [
    {{ "name": "field_name", "doc": "/// one-line or short doc...\n/// ..." }}
  ]
}}"#).ok();
    writeln!(s, "- `struct_doc`: A short 1–2 sentence rustdoc for the struct (above attributes).").ok();
    writeln!(s, "- `fields`: One entry **per named field** appearing in the struct body; the `doc` value must be a ready-to-insert `///` block for that field (keep it short, include units/invariants if relevant).").ok();

    s
}
