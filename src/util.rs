use crate::regexes::{re_attr, re_field};
use regex::Regex;

#[derive(Debug)]
pub struct FieldSpec {
    pub name: String,
    pub field_line0: usize,
    pub insert_line0: usize,
    pub parent_fqpath: String,
    pub field_line_text: String,
}

pub fn extract_lines(src: &str, lo_line0: usize, hi_line0: usize) -> String {
    src.lines()
        .enumerate()
        .filter(|(i, _)| *i >= lo_line0 && *i <= hi_line0)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n")
}

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
                if ch == '{' { open += 1; }
                if ch == '}' { open -= 1; }
            }
            if open == 0 {
                let (start, _) = brace_line_start.unwrap();
                return Some((start, i));
            }
        }
    }
    None
}

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
                let name = Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\s*:"#)
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
