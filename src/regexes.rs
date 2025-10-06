use regex::Regex;
use std::sync::OnceLock;

pub fn re_word() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}
pub fn re_struct() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\b"#).unwrap())
}
pub fn re_fn_sig() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\b"#).unwrap())
}
pub fn re_field() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?[A-Za-z_][A-Za-z0-9_]*\s*:\s*[^;{}]+,?\s*$"#).unwrap())
}
pub fn re_attr() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*#\["#).unwrap())
}

pub fn find_sig_line_near(src: &str, start_line0: usize, re: &Regex) -> Option<usize> {
    let total = src.lines().count();
    for i in start_line0.min(total)..(start_line0 + 20).min(total) {
        if src.lines().nth(i).map(|l| re.is_match(l)).unwrap_or(false) { return Some(i); }
    }
    let up_lo = start_line0.saturating_sub(5);
    for i in (up_lo..start_line0.min(total)).rev() {
        if src.lines().nth(i).map(|l| re.is_match(l)).unwrap_or(false) { return Some(i); }
    }
    None
}
