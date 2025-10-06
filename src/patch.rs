use crate::error::{Error, Result};
use crate::model::LlmDocResult;
use crate::regexes::{find_sig_line_near, re_field, re_fn_sig, re_struct};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub struct Edit { start: usize, end: usize, text: String }

fn apply_edits(mut text: String, mut edits: Vec<Edit>) -> String {
    edits.sort_by(|a, b| b.start.cmp(&a.start));
    for e in edits {
        if e.start <= e.end && e.end <= text.len() { text.replace_range(e.start..e.end, &e.text); }
    }
    text
}

pub enum InsertWhere { Before(usize), Replace(usize, usize) }

fn doc_slot_above_attrs(src: &str, struct_sig_line0: usize, overwrite: bool) -> Option<InsertWhere> {
    let lines: Vec<&str> = src.lines().collect();
    let mut attr_first = struct_sig_line0;
    let mut i = struct_sig_line0.saturating_sub(1);
    let mut saw_attr = false;
    while i < lines.len() {
        if i == usize::MAX { break; }
        let t = lines[i].trim_start();
        if t.starts_with("#[") || t.starts_with("#![") {
            saw_attr = true; attr_first = i;
            if i == 0 { break; }
            i = i.saturating_sub(1); continue;
        }
        if t.is_empty() && saw_attr {
            if i == 0 { break; }
            i = i.saturating_sub(1); continue;
        }
        break;
    }
    let anchor = attr_first;

    if anchor > 0 && lines[anchor - 1].trim_start().starts_with("///") {
        let mut doc_lo = anchor - 1;
        while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") { doc_lo -= 1; }
        if !overwrite { return None; }
        return Some(InsertWhere::Replace(doc_lo, anchor));
    }
    Some(InsertWhere::Before(anchor))
}

fn field_doc_slot(src: &str, insert_line0: usize, overwrite: bool) -> Option<InsertWhere> {
    let lines: Vec<&str> = src.lines().collect();
    if insert_line0 == 0 { return Some(InsertWhere::Before(0)); }
    let i = insert_line0 - 1;
    if lines.get(i).map_or(false, |l| l.trim_start().starts_with("///")) {
        if !overwrite { return None; }
        let mut doc_lo = i;
        while doc_lo > 0 && lines[doc_lo - 1].trim_start().starts_with("///") { doc_lo -= 1; }
        return Some(InsertWhere::Replace(doc_lo, insert_line0));
    }
    Some(InsertWhere::Before(insert_line0))
}

fn find_doc_insertion_range(source: &str, start_line_1: usize) -> (usize, usize) {
    let lines: Vec<&str> = source.lines().collect();
    let sig_idx = start_line_1.saturating_sub(1);
    let mut lo = sig_idx;

    let mut i = sig_idx.saturating_sub(1);
    while i < lines.len() {
        if i == usize::MAX { break; }
        let t = lines[i].trim_start();
        if t.starts_with("///") || t.starts_with("#![doc") || t.starts_with("#[doc") {
            lo = i; if i == 0 { break; }
            i = i.saturating_sub(1); continue;
        }
        break;
    }

    let mut j = sig_idx.saturating_sub(1);
    let mut saw_attr = false;
    let mut attr_first_idx = usize::MAX;
    while j < lines.len() {
        if j == usize::MAX { break; }
        let t = lines[j].trim_start();
        if t.starts_with("#[") || t.starts_with("#![") {
            saw_attr = true; attr_first_idx = j; lo = lo.min(j);
            if j == 0 { break; }
            j = j.saturating_sub(1); continue;
        }
        break;
    }

    if saw_attr {
        let hi = attr_first_idx; (lo, hi)
    } else {
        let mut lo2 = lo;
        if sig_idx > 0 && lines[sig_idx - 1].trim().is_empty() { lo2 = lo2.min(sig_idx - 1); }
        (lo2, sig_idx)
    }
}

fn indent_like(target_line: &str, doc: &str) -> String {
    let indent: String = target_line.chars().take_while(|c| c.is_whitespace()).collect();
    if indent.is_empty() {
        if doc.ends_with('\n') { doc.to_string() } else { format!("{doc}\n") }
    } else {
        let mut out = String::with_capacity(doc.len() + indent.len() * 4);
        for (i, line) in doc.lines().enumerate() {
            if i > 0 { out.push('\n'); }
            if line.starts_with("///") { out.push_str(&indent); out.push_str(line); }
            else { out.push_str(&indent); out.push_str("/// "); out.push_str(line); }
        }
        if !out.ends_with('\n') { out.push('\n'); }
        out
    }
}

pub fn patch_files_with_docs(results: &[LlmDocResult], overwrite: bool) -> Result<()> {
    let mut by_file: BTreeMap<&str, Vec<&LlmDocResult>> = BTreeMap::new();
    for r in results { by_file.entry(&r.file).or_default().push(r); }

    for (file, mut items) in by_file {
        let original = fs::read_to_string(file).map_err(|e| Error::Io { path: Some(PathBuf::from(file)), source: e })?;

        let mut line_starts: Vec<usize> = vec![0];
        for (i, b) in original.bytes().enumerate() { if b == b'\n' { line_starts.push(i + 1); } }
        line_starts.push(original.len());

        let mut edits: Vec<Edit> = Vec::new();
        let mut skipped_no_sig = 0usize;
        let mut skipped_existing_doc = 0usize;

        items.sort_by_key(|r| r.start_line.unwrap_or(0));
        for r in items {
            let Some(start_line_1) = r.start_line else { continue; };
            let start_line0 = start_line_1.saturating_sub(1) as usize;

            let re_for_kind = match r.kind.as_str() {
                "struct" => re_struct(),
                "field"  => re_field(),
                _        => re_fn_sig(),
            };
            let sig_line0_opt = if r.kind == "field" {
                Some(start_line0)
            } else {
                find_sig_line_near(&original, start_line0, re_for_kind)
            };

            let (ins_lo, ins_hi, indent_line_idx) = match (r.kind.as_str(), sig_line0_opt) {
                ("struct", Some(sig_line0)) => match doc_slot_above_attrs(&original, sig_line0, overwrite) {
                    Some(InsertWhere::Before(i))      => (i, i, i.min(sig_line0)),
                    Some(InsertWhere::Replace(lo,hi)) => (lo, hi, hi.min(sig_line0)),
                    None => { skipped_existing_doc += 1; continue; }
                },
                ("field", _) => match field_doc_slot(&original, start_line0, overwrite) {
                    Some(InsertWhere::Before(i))      => (i, i, i),
                    Some(InsertWhere::Replace(lo,hi)) => (lo, hi, hi),
                    None => { skipped_existing_doc += 1; continue; }
                },
                (_, Some(sig_line0)) => {
                    let (lo, hi) = find_doc_insertion_range(&original, sig_line0 + 1);
                    (lo, hi, sig_line0)
                }
                _ => { skipped_no_sig += 1; continue; }
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
            let end_b   = *line_starts.get(ins_hi).unwrap_or(&start_b);

            let target_line = original.lines().nth(indent_line_idx).unwrap_or("");
            let repl = indent_like(target_line, &r.llm_doc);

            edits.push(Edit { start: start_b, end: end_b, text: repl });
        }

        if edits.is_empty() {
            eprintln!("Patched {}: 0 edits (skipped_no_sig={}, skipped_existing_doc={})", file, skipped_no_sig, skipped_existing_doc);
            continue;
        }

        let new_text = apply_edits(original, edits);
        fs::write(file, new_text).map_err(|e| Error::Io { path: Some(PathBuf::from(file)), source: e })?;
    }

    Ok(())
}
