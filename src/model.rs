use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Span {
    pub start_line: Option<u32>,
    pub end_line:   Option<u32>,
    pub start_byte: Option<u64>,
    pub end_byte:   Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Row {
    pub kind: String,
    pub name: String,
    #[serde(rename = "crate", alias = "crate_", alias = "crate_field")]
    pub crate_name: Option<String>,
    pub module_path: Option<Vec<String>>,
    pub fqpath: String,
    pub visibility: String,
    pub file: String,
    pub span: Span,
    pub signature: String,
    pub has_body: bool,
    pub doc: Option<String>,
    pub body_text: Option<String>,
    pub callers: Option<Vec<String>>,
}

impl Row {
    pub fn had_doc(&self) -> bool {
        self.doc.as_deref().map_or(false, |d| !d.trim().is_empty())
    }
    pub fn span_bytes(&self) -> (u64, u64) {
        (
            self.span.start_byte.unwrap_or(0),
            self.span.end_byte.unwrap_or(u64::MAX),
        )
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct LlmDocResult {
    pub kind: String,
    pub fqpath: String,
    pub file: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub signature: String,
    pub callers: Vec<String>,
    pub referenced_symbols: Vec<String>,
    pub llm_doc: String,
    pub had_existing_doc: bool,
}

#[derive(Debug, Deserialize)]
pub struct FieldDocOut { pub name: String, pub doc: String }

#[derive(Debug, Deserialize)]
pub struct StructDocResponse {
    pub struct_doc: String,
    pub fields: Vec<FieldDocOut>,
}

// Utility: find referencing functions
use regex::Regex;

pub fn referencing_functions(struct_name: &str, struct_fq: &str, fns: &[&Row]) -> Vec<String> {
    let word_name = Regex::new(&format!(r"\b{}\b", regex::escape(struct_name))).unwrap();
    let word_fq   = Regex::new(&regex::escape(struct_fq)).unwrap();

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

pub fn collect_symbol_refs(body: &str, all_symbols: &BTreeSet<String>, word_re: &Regex) -> Vec<String> {
    if body.is_empty() { return vec![]; }
    let mut found = BTreeSet::new();
    for m in word_re.find_iter(body) {
        let w = m.as_str();
        if all_symbols.contains(w) {
            found.insert(w.to_string());
            if found.len() == 64 { break; }
        }
    }
    found.into_iter().collect()
}
