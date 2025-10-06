use regex::Regex;

pub fn sanitize_llm_doc(raw: &str) -> String {
    let s = strip_xml_like(raw, "think");
    let s = strip_wrapper_markers(&s, &["ANSWER:", "RESPONSE:", "OUTPUT:", "QUESTION:"]);
    let s = unwrap_code_fence_if_wrapped(&s);
    let s = decode_common_escapes(&s);
    let s = coerce_to_rustdoc(&s);
    let s = balance_code_fences(&s);
    strip_leading_empty_doc_lines(&s)
}

fn strip_xml_like(s: &str, tag: &str) -> String {
    let re = Regex::new(&format!(r"(?is)<\s*{}\b[^>]*>.*?</\s*{}\s*>", regex::escape(tag), regex::escape(tag))).unwrap();
    re.replace_all(s, "").trim().to_string()
}

fn strip_wrapper_markers(s: &str, markers: &[&str]) -> String {
    let mut in_fence = false;
    let mut byte_pos = 0usize;
    let mut split: Option<usize> = None;
    for line in s.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") { in_fence = !in_fence; }
        if !in_fence {
            if let Some(m) = markers.iter().find(|m| trimmed.starts_with(**m)) {
                let left_trim = line.len() - trimmed.len();
                split = Some(byte_pos + left_trim + m.len());
            }
        }
        byte_pos += line.len();
        if byte_pos < s.len() { byte_pos += 1; }
    }
    if let Some(idx) = split { s[idx..].trim().to_string() } else { s.trim().to_string() }
}

fn unwrap_code_fence_if_wrapped(s: &str) -> String {
    let mut fence_count = 0usize;
    let mut lines = Vec::new();
    for line in s.lines() {
        let l = line.trim_end();
        if l.starts_with("```") { fence_count += 1; }
        lines.push(l);
    }
    if fence_count == 2
        && lines.first().map(|l| l.starts_with("```")).unwrap_or(false)
        && lines.last().map(|l| l.starts_with("```")).unwrap_or(false)
    {
        return lines[1..lines.len() - 1].join("\n");
    }
    s.trim_matches('`').trim().to_string()
}

fn decode_common_escapes(s: &str) -> String {
    let mut t = s.to_string();
    t = t.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t");
    t = t.replace("\\\"", "\"");
    t = t.replace("\\\\n", "\n").replace("\\\\t", "\t").replace("\\\\\"", "\"");
    t
}

fn extract_longest_doc_block(lines: &[String]) -> Vec<String> {
    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut cur_start = None::<usize>;
    let mut cur_len = 0usize;

    let is_doc = |s: &str| s.trim_start().starts_with("///");
    for (i, l) in lines.iter().enumerate() {
        if is_doc(l) {
            if cur_start.is_none() { cur_start = Some(i); cur_len = 0; }
            cur_len += 1;
        } else if let Some(st) = cur_start {
            if cur_len > best_len { best_len = cur_len; best_start = st; }
            cur_start = None; cur_len = 0;
        }
    }
    if let Some(st) = cur_start {
        if cur_len > best_len { best_len = cur_len; best_start = st; }
    }

    if best_len == 0 {
        let first = lines.iter().find(|l| !l.trim().is_empty()).cloned().unwrap_or_else(|| "///".into());
        return vec![if first.starts_with("///") { first } else { format!("/// {}", first) }];
    }
    lines[best_start..best_start + best_len].to_vec()
}

fn coerce_to_rustdoc(raw: &str) -> String {
    let mut lines: Vec<String> = raw.replace('\r', "")
        .lines().map(|l| l.trim_end().to_string()).collect();

    if lines.iter().all(|l| l.trim().is_empty()) { return String::new(); }

    for l in &mut lines {
        match l.trim() {
            "Parameters:" => *l = "## Parameters".into(),
            "Returns:"    => *l = "## Returns".into(),
            "Errors:"     => *l = "## Errors".into(),
            "Safety:"     => *l = "## Safety".into(),
            "Notes:"      => *l = "## Notes".into(),
            "Examples:"   => *l = "## Examples".into(),
            _ => {}
        }
    }

    let mut coerced: Vec<String> = Vec::with_capacity(lines.len());
    let mut prev_blank = false;
    for mut t in lines {
        if t.starts_with("```") && !t.starts_with("///") { continue; }
        t = t.trim().to_string();
        let is_blank = t.is_empty();
        if is_blank {
            if prev_blank { continue; }
            prev_blank = true; coerced.push("///".into()); continue;
        }
        prev_blank = false;

        if t.starts_with("///") { coerced.push(t); continue; }

        if t == "{" || t == "}" || t == "}," || t.ends_with(':') || t.ends_with("\":") { continue; }

        if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
            t = t[1..t.len() - 1].to_string();
        }
        coerced.push(format!("/// {}", t));
    }

    let doc_block = extract_longest_doc_block(&coerced);
    let mut out: Vec<String> = Vec::with_capacity(doc_block.len());
    let mut fence_depth = 0usize;
    for mut l in doc_block {
        if l.ends_with('\\') && !l.ends_with("\\\\") { l.pop(); }
        let t = l.trim_start_matches('/').trim_start();
        if t.starts_with("```") {
            if fence_depth == 0 && t == "```" { l = "/// ```rust".into(); }
            fence_depth ^= 1;
        }
        out.push(l);
    }
    if fence_depth == 1 { out.push("/// ```".into()); }

    while matches!(out.last().map(|s| s.trim_end()), Some("///") | Some("")) { out.pop(); }

    out.join("\n")
}

fn balance_code_fences(s: &str) -> String {
    let mut depth = 0i32;
    for l in s.lines() {
        if l.trim_start().trim_start_matches('/').trim_start().starts_with("```") { depth ^= 1; }
    }
    if depth == 1 { format!("{s}\n/// ```") } else { s.to_string() }
}

fn strip_leading_empty_doc_lines(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() && lines[i].trim_end() == "///" { i += 1; }
    if i == 0 { return s.to_string(); }
    lines[i..].join("\n")
}
