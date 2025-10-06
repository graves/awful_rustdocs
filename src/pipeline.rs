use crate::error::{Error, Result};
use crate::model::{LlmDocResult, Row, StructDocResponse};
use crate::model::{collect_symbol_refs, referencing_functions};
use crate::grep::{calls_in_function_span, qualified_paths_in_span};
use crate::prompt::{build_markdown_question, build_struct_request_with_refs};
use crate::sanitize::sanitize_llm_doc;
use crate::regexes::re_word;

use awful_aj::api;
use awful_aj::template::ChatTemplate;
use awful_aj::config::AwfulJadeConfig;

use std::collections::{BTreeMap, BTreeSet};

pub struct Ctx {
    pub cfg: AwfulJadeConfig,
    pub tpl_fn: ChatTemplate,
    pub tpl_struct: ChatTemplate,
    pub opts: crate::cli::GenerateOpts,
}

pub struct Pipeline<'a> {
    pub ctx: &'a Ctx,
    pub rows: Vec<Row>,
    pub all_symbols: BTreeSet<String>,
    pub fn_rows: Vec<Row>,
}

impl<'a> Pipeline<'a> {
    pub fn from_harvest(ctx: &'a Ctx, rows: Vec<Row>) -> Self {
        let all_symbols = rows.iter().map(|r| r.name.clone()).filter(|s| !s.is_empty()).collect();
        let fn_rows     = rows.iter().filter(|r| r.kind == "fn").cloned().collect();
        Self { ctx, rows, all_symbols, fn_rows }
    }

    pub fn wanted<'b>(&'b self) -> impl Iterator<Item = &'b Row> {
        let only = &self.ctx.opts.only;
        self.rows.iter().filter(move |r| {
            (r.kind == "fn" || r.kind == "struct")
                && (only.is_empty() || only.iter().any(|s| s == &r.name || s == &r.fqpath))
        })
    }
}

pub async fn run_generation<'a>(
    ctx: &'a Ctx,
    rows: Vec<Row>,
) -> Result<Vec<LlmDocResult>> {
    let pipe = Pipeline::from_harvest(ctx, rows);

    // group by file for stable traversal
    let mut per_file: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for r in pipe.wanted() {
        per_file.entry(r.file.clone()).or_default().push(r.clone());
    }
    for v in per_file.values_mut() {
        v.sort_by_key(|r| (r.span.start_line.unwrap_or(0), r.fqpath.clone()));
    }
    if per_file.is_empty() && !ctx.opts.only.is_empty() {
        eprintln!("No items matched --only filter: {}", ctx.opts.only.join(", "));
    }

    let fn_rows_refs: Vec<&Row> = pipe.fn_rows.iter().collect();
    let runner = crate::runner::ProcRunner;

    let mut all_results: Vec<LlmDocResult> = Vec::new();
    let mut processed = 0usize;

    'files: for (_file, items) in per_file.iter() {
        for item in items {
            if let Some(limit) = ctx.opts.limit {
                if processed >= limit { break 'files; }
            }
            processed += 1;

            let had_existing_doc = item.had_doc();
            if had_existing_doc && !ctx.opts.overwrite {
                if item.kind != "struct" { continue; }
            }

            match item.kind.as_str() {
                "fn" => {
                    let mut referenced_symbols = collect_symbol_refs(
                        item.body_text.as_deref().unwrap_or(""),
                        &pipe.all_symbols,
                        re_word(),
                    );

                    let (start_b, end_b) = item.span_bytes();

                    if !ctx.opts.no_paths {
                        let qpaths = qualified_paths_in_span(&runner, &item.file, start_b, end_b).unwrap_or_default();
                        referenced_symbols.extend(qpaths.into_iter());
                    }

                    let calls_in_span = if ctx.opts.no_calls {
                        vec![]
                    } else {
                        calls_in_function_span(&runner, &item.file, start_b, end_b).unwrap_or_default()
                    };

                    let question = build_markdown_question(item, &referenced_symbols, &calls_in_span);

                    let answer = api::ask(&ctx.cfg, question, &ctx.tpl_fn, None, None)
                        .await
                        .map_err(|e| Error::External {
                            context: "LLM ask() failed",
                            message: format!("{}: {}", item.fqpath, e),
                        })?;

                    let llm_doc_block = sanitize_llm_doc(&answer);

                    all_results.push(LlmDocResult {
                        kind: "fn".into(),
                        fqpath: item.fqpath.clone(),
                        file: item.file.clone(),
                        start_line: item.span.start_line,
                        end_line: item.span.end_line,
                        signature: item.signature.clone(),
                        callers: item.callers.clone().unwrap_or_default(),
                        referenced_symbols,
                        llm_doc: llm_doc_block,
                        had_existing_doc,
                    });
                }

                "struct" => {
                    // load file + find struct body
                    let file_src = std::fs::read_to_string(&item.file).map_err(|e| Error::Io {
                        path: Some(std::path::PathBuf::from(&item.file)),
                        source: e,
                    })?;

                    let approx_line0 = item.span.start_line.unwrap_or(1).saturating_sub(1) as usize;
                    let struct_sig0 = match crate::regexes::find_sig_line_near(&file_src, approx_line0, crate::regexes::re_struct()) {
                        Some(l) => l,
                        None => { eprintln!("warn: could not locate struct sig for {}", item.fqpath); continue; }
                    };
                    let (body_lo, body_hi) = match crate::util::find_struct_body_block(&file_src, struct_sig0) {
                        Some(p) => p,
                        None => { eprintln!("warn: could not locate struct body for {}", item.fqpath); continue; }
                    };
                    let body_text = crate::util::extract_lines(&file_src, body_lo, body_hi);

                    // references
                    let refs = referencing_functions(&item.name, &item.fqpath, &fn_rows_refs);

                    // ask / parse
                    let question = build_struct_request_with_refs(item, &body_text, &refs);
                    let raw = api::ask(&ctx.cfg, question, &ctx.tpl_struct, None, None)
                        .await
                        .map_err(|e| Error::External {
                            context: "LLM ask() failed",
                            message: format!("{}: {}", item.fqpath, e),
                        })?;

                    let parsed: Result<StructDocResponse> =
                        serde_json::from_str(&raw).map_err(|e| Error::Json { context: "struct JSON parse", source: e });

                    let (struct_doc, field_docs) = match parsed {
                        Ok(v) => (v.struct_doc, v.fields),
                        Err(err) => { eprintln!("{}", err); (raw, vec![]) }
                    };

                    let struct_llm_doc = crate::sanitize::sanitize_llm_doc(&struct_doc);

                    // map fields
                    let fields_in_file = crate::util::extract_struct_fields_in_file(
                        &file_src, body_lo, body_hi, &item.fqpath,
                    );
                    let mut field_index: BTreeMap<String, (usize, String)> = BTreeMap::new();
                    for f in fields_in_file { field_index.insert(f.name, (f.insert_line0, f.field_line_text)); }

                    all_results.push(LlmDocResult {
                        kind: "struct".into(),
                        fqpath: item.fqpath.clone(),
                        file: item.file.clone(),
                        start_line: item.span.start_line,
                        end_line: item.span.end_line,
                        signature: item.signature.clone(),
                        callers: item.callers.clone().unwrap_or_default(),
                        referenced_symbols: vec![],
                        llm_doc: struct_llm_doc,
                        had_existing_doc,
                    });

                    for fd in field_docs {
                        if let Some((insert0, field_line_text)) = field_index.get(&fd.name).cloned() {
                            let doc_block = crate::sanitize::sanitize_llm_doc(&fd.doc);
                            all_results.push(LlmDocResult {
                                kind: "field".into(),
                                fqpath: format!("{}::{}", item.fqpath, fd.name),
                                file: item.file.clone(),
                                start_line: Some((insert0 as u32) + 1),
                                end_line: None,
                                signature: field_line_text,
                                callers: vec![],
                                referenced_symbols: vec![],
                                llm_doc: doc_block,
                                had_existing_doc: false,
                            });
                        } else {
                            eprintln!("warn: field '{}' not found in {} — skipping doc", fd.name, item.fqpath);
                        }
                    }
                }

                _ => {}
            }
        }
    }

    Ok(all_results)
}
