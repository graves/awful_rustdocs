use crate::error::{Error, Result};
use crate::grep::{calls_in_function_span, qualified_paths_in_span};
use crate::model::{LlmDocResult, Row, StructDocResponse};
use crate::model::{collect_symbol_refs, referencing_functions};
use crate::prompt::{build_markdown_question, build_struct_request_with_refs};
use crate::regexes::re_word;
use crate::sanitize::sanitize_llm_doc;

use awful_aj::api;
use awful_aj::config::AwfulJadeConfig;
use awful_aj::template::ChatTemplate;
use tracing::{debug, error, info, info_span, instrument, warn};

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// Context container for the generation pipeline, holding configuration, templates, and generation options.
/// Used across the pipeline to maintain state and enable consistent message formatting and behavior.
pub struct Ctx {
    /// Configuration settings for the Awful Jade system.
    pub cfg: AwfulJadeConfig,
    /// Function to render chat messages using a template (for user messages).
    /// Must be callable during generation to format prompts.
    pub tpl_fn: ChatTemplate,
    /// Function to render structured data using a template (for structured outputs).
    /// Used when generating structured responses like JSON or tables.
    pub tpl_struct: ChatTemplate,
    /// Command-line options used to control generation behavior (e.g., max tokens, temperature).
    /// Passed from CLI to influence output parameters.
    pub opts: crate::cli::GenerateOpts,
}

/// A pipeline that processes data through stages, maintaining context and state across rows and symbols.
pub struct Pipeline<'a> {
    /// Context reference for the pipeline execution.
    pub ctx: &'a Ctx,
    /// Rows of data processed in the current pipeline stage.
    pub rows: Vec<Row>,
    /// Set of all unique symbol names encountered during processing.
    pub all_symbols: BTreeSet<String>,
    /// Function rows (e.g., generated or transformed rows) for functional processing.
    pub fn_rows: Vec<Row>,
}

impl<'a> Pipeline<'a> {
    /// Constructs a new [`Pipeline`] from a collection of [`Row`] entries harvested from a data source.
    ///
    /// This function processes the provided `rows` by extracting symbol names (filtering out empty ones)
    /// and grouping function-like rows (`kind == "fn"`). It then initializes a [`Pipeline`] with the
    /// context, raw rows, collected symbols, and function rows for later processing.
    ///
    /// # Parameters
    /// - `ctx`: A reference to the execution context containing configuration and state.
    /// - `rows`: A vector of [`Row`] entries representing harvested data, each with a `name` and `kind`.
    ///
    /// # Returns
    /// A newly constructed [`Pipeline`] instance containing the processed data.
    ///
    /// # Notes
    /// - The `all_symbols` field collects non-empty `name` fields from all rows.
    /// - The `fn_rows` field collects only rows where `kind` is `"fn"`, preserving their original data.
    /// - This function does not perform any I/O or side effects beyond data aggregation.
    ///
    /// # Examples
    /// ```no_run
    /// use crate::pipeline::Pipeline;
    /// use crate::Ctx;
    /// use crate::Row;
    ///
    /// let ctx = Ctx::default();
    /// let rows = vec![
    ///     Row { name: "x".into(), kind: "var".into() },
    ///     Row { name: "y".into(), kind: "fn".into() },
    ///     Row { name: "".into(), kind: "var".into() },
    /// ];
    ///
    /// Pipeline::from_harvest(&ctx, rows)
    /// ```
    pub fn from_harvest(ctx: &'a Ctx, rows: Vec<Row>) -> Self {
        let all_symbols = rows
            .iter()
            .map(|r| r.name.clone())
            .filter(|s| !s.is_empty())
            .collect();
        let fn_rows = rows.iter().filter(|r| r.kind == "fn").cloned().collect();
        Self {
            ctx,
            rows,
            all_symbols,
            fn_rows,
        }
    }

    /// Returns an iterator over rows that match the specified criteria: either have a kind of "fn" or "struct", and optionally match a name or full qualified path in the `only` list.
    /// If `only` is empty, all rows with the specified kinds are included.
    ///
    /// Parameters:
    /// - `self`: The `Pipeline<'a>` instance containing the rows and context options.
    ///
    /// Returns:
    /// - An iterator over references to `Row` that match the filtering conditions.
    ///
    /// Notes:
    /// - The filtering is based on the `kind` field of the row, which must be either "fn" or "struct".
    /// - If `only` is provided, the row's `name` or `fqpath` must match one of the strings in `only`.
    /// - The `only` list is checked for exact matches using `&r.name` or `&r.fqpath`.
    ///
    /// Examples:
    /// ```rust
    /// use crate::pipeline::Pipeline;
    /// use crate::Row;
    ///
    /// let rows = vec![
    ///     Row { kind: "fn", name: "foo", fqpath: "foo::bar" },
    ///     Row { kind: "struct", name: "baz", fqpath: "baz::qux" },
    /// ];
    ///
    /// let ctx = crate::context::Context { opts: crate::context::Opts { only: vec!["foo".into()] }, ..Default::default() };
    /// let pipeline = Pipeline { rows, ctx };
    /// let filtered = pipeline.wanted();
    ///
    /// assert!(filtered.any());
    /// assert!(!filtered.any()); // if only contains "bar", then no match
    /// ```
    pub fn wanted<'b>(&'b self) -> impl Iterator<Item = &'b Row> {
        let only = &self.ctx.opts.only;
        self.rows.iter().filter(move |r| {
            (r.kind == "fn" || r.kind == "struct")
                && (only.is_empty() || only.iter().any(|s| s == &r.name || s == &r.fqpath))
        })
    }
}

/// Runs the generation of Rust documentation for symbols (functions and structs) based on provided rows of code metadata.
/// For each symbol, it extracts relevant context, builds a question using references and call chains, and sends it to the LLM via `api::ask`.
/// The results are sanitized and stored in `LlmDocResult` format, grouped by file and processed in order of line position.
/// If a symbol already has documentation and `--overwrite` is not specified, it is skipped unless it's a struct.
/// Function execution includes timing and logging for performance and debugging.
///
/// # Parameters
/// - `ctx`: A reference to the execution context containing configuration, templates, and runtime state.
/// - `rows`: A vector of `Row` entries representing code symbols (functions, structs) with metadata like file, span, and kind.
///
/// # Returns
/// A `Result<Vec<LlmDocResult>>` containing the generated documentation for each symbol, or an error if generation fails.
///
/// # Errors
/// - Returns `Error::Io` when reading file content fails.
/// - Returns `Error::Json` when parsing LLM-generated JSON response fails.
/// - Returns `Error::External` when the LLM API call fails.
/// - Returns `Error::External` if no struct signature or body is found in the source file.
///
/// # Notes
/// - Processing stops early if `--limit` is reached.
/// - Existing documentation is skipped for non-struct symbols unless `--overwrite` is enabled.
/// - Structs require parsing of the source file to locate their signature and body block.
/// - Symbol references and function calls are collected using regex and span analysis.
/// - All LLM requests use the configured template (function or struct) and are passed through the `api::ask` layer.
#[instrument(level = "info", skip(ctx, rows))]
pub async fn run_generation<'a>(ctx: &'a Ctx, rows: Vec<Row>) -> Result<Vec<LlmDocResult>> {
    debug!(rows = rows.len(), "generation started");

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
        warn!(only = %ctx.opts.only.join(", "), "no items matched --only filter");
    }

    let fn_rows_refs: Vec<&Row> = pipe.fn_rows.iter().collect();
    let runner = crate::runner::ProcRunner;

    let mut all_results: Vec<LlmDocResult> = Vec::new();
    let mut processed = 0usize;

    'files: for (file, items) in per_file.iter() {
        let _file_span = info_span!("file", file = %file).entered();
        debug!(items = items.len(), "begin file");

        for item in items {
            let _sym_span = info_span!(
                "symbol",
                kind = %item.kind,
                symbol = %item.fqpath,
                file = %item.file,
                start_line = ?item.span.start_line,
                end_line = ?item.span.end_line
            )
            .entered();

            let t_symbol = Instant::now();
            debug!("begin processing symbol");

            if let Some(limit) = ctx.opts.limit {
                if processed >= limit {
                    info!(limit, "limit reached, stopping generation");
                    break 'files;
                }
            }
            processed += 1;

            let had_existing_doc = item.had_doc();
            if had_existing_doc && !ctx.opts.overwrite {
                if item.kind != "struct" {
                    let elapsed_ms = t_symbol.elapsed().as_millis();
                    info!(
                        elapsed_ms,
                        "skipping: existing rustdoc present (use --overwrite to replace)"
                    );
                    continue;
                }
                // structs still proceed to allow field docs via single LLM call
            }

            match item.kind.as_str() {
                "fn" => {
                    info!("generating docs for function");

                    let mut referenced_symbols = collect_symbol_refs(
                        item.body_text.as_deref().unwrap_or(""),
                        &pipe.all_symbols,
                        re_word(),
                    );

                    let (start_b, end_b) = item.span_bytes();

                    if !ctx.opts.no_paths {
                        let qpaths = qualified_paths_in_span(&runner, &item.file, start_b, end_b)
                            .unwrap_or_default();
                        referenced_symbols.extend(qpaths.into_iter());
                    }

                    let calls_in_span = if ctx.opts.no_calls {
                        vec![]
                    } else {
                        calls_in_function_span(&runner, &item.file, start_b, end_b)
                            .unwrap_or_default()
                    };

                    let question =
                        build_markdown_question(item, &referenced_symbols, &calls_in_span);
                    debug!(question_len = question.len(), "sending LLM request (fn)");

                    let t_llm = Instant::now();
                    let answer = api::ask(&ctx.cfg, question, &ctx.tpl_fn, None, None)
                        .await
                        .map_err(|e| {
                            error!(error = %e, fqpath = %item.fqpath, "LLM ask() failed");
                            Error::External {
                                context: "LLM ask() failed",
                                message: format!("{}: {}", item.fqpath, e),
                            }
                        })?;
                    let llm_ms = t_llm.elapsed().as_millis();

                    debug!(
                        answer_len = answer.len(),
                        llm_ms, "received LLM response (fn)"
                    );
                    let llm_doc_block = sanitize_llm_doc(&answer);
                    info!(
                        doc_lines = llm_doc_block.lines().count(),
                        elapsed_ms = t_symbol.elapsed().as_millis(),
                        llm_ms,
                        "sanitized rustdoc (fn)"
                    );

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
                    info!("generating docs for struct and its fields");

                    // load file + find struct body
                    let file_src = std::fs::read_to_string(&item.file).map_err(|e| Error::Io {
                        path: Some(std::path::PathBuf::from(&item.file)),
                        source: e,
                    })?;

                    let approx_line0 = item.span.start_line.unwrap_or(1).saturating_sub(1) as usize;
                    let struct_sig0 = match crate::regexes::find_sig_line_near(
                        &file_src,
                        approx_line0,
                        crate::regexes::re_struct(),
                    ) {
                        Some(l) => l,
                        None => {
                            warn!("could not locate struct sig");
                            continue;
                        }
                    };
                    let (body_lo, body_hi) =
                        match crate::util::find_struct_body_block(&file_src, struct_sig0) {
                            Some(p) => p,
                            None => {
                                warn!("could not locate struct body");
                                continue;
                            }
                        };
                    let body_text = crate::util::extract_lines(&file_src, body_lo, body_hi);

                    // references
                    let refs = referencing_functions(&item.name, &item.fqpath, &fn_rows_refs);

                    // ask / parse
                    let question = build_struct_request_with_refs(item, &body_text, &refs);
                    debug!(
                        question_len = question.len(),
                        refs = refs.len(),
                        "sending LLM request (struct)"
                    );

                    let t_llm = Instant::now();
                    let raw = api::ask(&ctx.cfg, question, &ctx.tpl_struct, None, None)
                        .await
                        .map_err(|e| {
                            error!(error = %e, fqpath = %item.fqpath, "LLM ask() failed");
                            Error::External {
                                context: "LLM ask() failed",
                                message: format!("{}: {}", item.fqpath, e),
                            }
                        })?;
                    let llm_ms = t_llm.elapsed().as_millis();

                    debug!(
                        answer_len = raw.len(),
                        llm_ms, "received LLM response (struct)"
                    );

                    let parsed: Result<StructDocResponse> =
                        serde_json::from_str(&raw).map_err(|e| Error::Json {
                            context: "struct JSON parse",
                            source: e,
                        });

                    let (struct_doc, field_docs) = match parsed {
                        Ok(v) => {
                            info!(fields = v.fields.len(), "parsed struct JSON");
                            (v.struct_doc, v.fields)
                        }
                        Err(err) => {
                            warn!(error = %err, "struct JSON parse failed; using raw payload");
                            (raw, vec![])
                        }
                    };

                    let struct_llm_doc = sanitize_llm_doc(&struct_doc);

                    // map fields
                    let fields_in_file = crate::util::extract_struct_fields_in_file(
                        &file_src,
                        body_lo,
                        body_hi,
                        &item.fqpath,
                    );
                    let mut field_index: BTreeMap<String, (usize, String)> = BTreeMap::new();
                    for f in fields_in_file {
                        field_index.insert(f.name, (f.insert_line0, f.field_line_text));
                    }

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
                        if let Some((insert0, field_line_text)) = field_index.get(&fd.name).cloned()
                        {
                            let doc_block = sanitize_llm_doc(&fd.doc);
                            debug!(field = %fd.name, insert_line = insert0 + 1, "prepared field doc");
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
                            warn!(field = %fd.name, "field not found in struct body; skipping doc");
                        }
                    }

                    info!(
                        elapsed_ms = t_symbol.elapsed().as_millis(),
                        llm_ms, "completed struct generation"
                    );
                }

                _ => {
                    let elapsed_ms = t_symbol.elapsed().as_millis();
                    debug!(kind = %item.kind, elapsed_ms, "unsupported symbol kind, skipping");
                }
            }

            debug!(
                elapsed_ms = t_symbol.elapsed().as_millis(),
                "finished processing symbol"
            );
        }

        debug!("finished file");
    }

    info!(generated = all_results.len(), "generation finished");
    Ok(all_results)
}
