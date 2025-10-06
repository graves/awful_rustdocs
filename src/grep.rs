use crate::error::Result;
use crate::runner::ToolRunner;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
pub struct SgRecord {
    pub file: String,
    #[serde(rename = "range")]
    pub range: SgRange,
    pub text: Option<String>,
    #[serde(default)]
    pub metaVariables: SgMetaVars,
}
#[derive(Debug, Deserialize)]
pub struct SgRange {
    #[serde(rename = "byteOffset")]
    pub byte: SgByteRange,
}
#[derive(Debug, Deserialize)]
pub struct SgByteRange {
    pub start: u64,
    pub end: u64,
}
#[derive(Debug, Default, Deserialize)]
pub struct SgMetaVars {
    #[serde(default)]
    pub single: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CallSite {
    pub kind: String,        // "plain" | "qualified" | "method"
    pub qual: Option<String>,
    pub callee: String,
}

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
            "run", "-l", "rust", "-p", pattern,
            "--json=stream", "--heading=never", "--color=never",
            file,
        ],
    )?;
    let mut out = Vec::new();
    for line in lines {
        let rec: SgRecord = serde_json::from_str(&line)
            .map_err(|e| crate::error::Error::Json { context: "ast-grep line", source: e })?;
        if rec.range.byte.start >= start && rec.range.byte.end <= end {
            out.push(rec);
        }
    }
    Ok(out)
}

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
            let name = mv.pointer("/N/text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if name.is_empty() { continue; }
            let qual = match kind {
                "qualified" => mv.pointer("/Q/text").and_then(|v| v.as_str()).map(|s| s.to_string()),
                "method"    => mv.pointer("/RECV/text").and_then(|v| v.as_str()).map(|s| s.to_string()),
                _ => None,
            };
            out.push(CallSite { kind: kind.to_string(), qual, callee: name });
        }
    }
    Ok(out)
}

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
                if t.contains("::") { paths.insert(t.to_string()); }
            }
        }
    }
    Ok(paths)
}
