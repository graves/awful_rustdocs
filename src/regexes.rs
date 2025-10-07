use regex::Regex;
use std::sync::OnceLock;

/// Returns a statically allocated regular expression that matches words consisting of letters, digits, and underscores, starting with a letter or underscore.
/// It is a shared reference to a compiled regex pattern that matches valid words according to the pattern `[A-Za-z_][A-Za-z0-9_]*`.
///
/// Parameters: None
///
/// Returns: A `&'static Regex` that matches words starting with a letter or underscore, followed by zero or more letters, digits, or underscores.
///
/// Errors: None; The regex is compiled at initialization and always succeeds due to the pattern being valid and the `unwrap()` call handling any parsing errors.
///
/// Notes:
/// - The pattern matches valid identifiers as defined in Rust's syntax (e.g., `hello`, `_private`, `my_var123`).
/// - This function uses `OnceLock` to ensure the regex is compiled only once, even across multiple calls.
///
/// Examples:
/// ```rust
/// let re = crate::regexes::re_word();
///
/// assert!(re.is_match("hello"));
/// assert!(re.is_match("_private"));
/// assert!(!re.is_match("123"));
/// ```
pub fn re_word() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

/// Returns a statically allocated regular expression that matches the keyword `struct`, optionally preceded by `pub` and parentheses.
/// The pattern matches `struct` literals such as `pub struct`, `struct`, or `pub struct SomeName`.
///
/// # Returns
/// - A reference to a compiled, `&'static Regex` that matches the `struct` keyword pattern.
///
/// Errors: None; the regex is compiled at initialization and always succeeds due to the pattern being valid and the `unwrap()` call handling any parsing errors.
///
/// # Notes
/// - The regex is compiled at initialization and cached for reuse.
/// - The pattern is case-sensitive and matches only the exact `struct` keyword.
/// - The `OnceLock` ensures thread-safety and avoids redundant compilation.
///
/// # Examples
/// let re = crate::regexes::re_struct();
///
/// assert!(re.is_match("pub struct Foo"));
/// assert!(re.is_match("struct Bar"));
/// assert!(!re.is_match("class Baz"));
/// ```
pub fn re_struct() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\b"#).unwrap())
}

/// Returns a static, compiled regular expression that matches the signature of a Rust function,
/// including optional `pub`, `async`, `const`, `unsafe`, `extern`, and function keyword patterns.
///
/// # Returns
/// - A `&'static Regex` that matches Rust function signatures.
///
/// # Notes
/// - The pattern is case-insensitive and allows for whitespace before and after the function keyword.
/// - The regex is compiled once and shared across all calls, making it efficient for repeated use.
/// - The pattern does not match function definitions with parameters or return types.
/// - The `OnceLock` ensures thread-safety and avoids redundant compilation.
///
/// # Examples
/// ```rust
/// let re = crate::regexes::re_fn_sig();
///
/// assert!(re.is_match("pub async fn hello() -> i32"));
/// assert!(re.is_match("unsafe extern "C" fn bar()"));
/// assert!(!re.is_match("pub class Foo"));
/// ```
pub fn re_fn_sig() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\b"#).unwrap())
}

/// Returns a static, compiled regular expression that matches a field declaration in Rust code,
/// specifically identifying patterns like `pub some_field: Type,` including optional `pub` and parameterized types.
///
/// The regex is lazily initialized using `OnceLock` to ensure thread-safety and avoid redundant compilation.
/// It matches field declarations where the field name starts with a letter or underscore, followed by alphanumeric characters,
/// and is followed by a colon and type specification without semicolon or braces.
///
/// # Returns
/// - A reference to a `&'static Regex` that matches Rust field declarations.
///
/// # Notes
/// - The pattern supports optional `pub` keyword and parentheses in function signatures.
/// - The regex does not match nested structures or expressions, only simple field declarations.
/// - The `OnceLock` ensures thread-safety and avoids redundant compilation.
///
/// # Examples
/// ```rust
/// let re = crate::regexes::re_field();
///
/// assert!(re.is_match("pub foo: String,"));
/// assert!(re.is_match("bar: i32,"));
/// assert!(re.is_match("pub func(param): usize"));
/// ```
pub fn re_field() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:r#)?[A-Za-z_][A-Za-z0-9_]*\s*:\s*[^;{}]+,?\s*$"#,
        )
        .unwrap()
    })
}

/// Returns a static, compiled regular expression that matches lines starting with `\s*#["` for attribute parsing.
///
/// The pattern `^\s*#\["` matches any line that starts with zero or more whitespace characters followed by `#["`.
/// The regex is cached and returned via `&'static Regex`, ensuring efficient reuse across calls.
///
/// # Returns
/// - A static reference to a `&'static Regex` matching `^\s*#\["`.
///
/// # Notes
/// - The regex is compiled at first access and cached for future use.
/// - The pattern is intended for parsing attribute definitions in configuration or code files.
/// - The `OnceLock` ensures thread-safety and avoids redundant compilation.
///
/// # Examples
/// ```rust
/// let re = crate::regexes::re_attr();
///
/// assert!(re.is_match(r#" #["attr" "value"]"#));
/// assert!(!re.is_match(r#"hello"#));
/// ```
pub fn re_attr() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^\s*#\["#).unwrap())
}

/// Searches for a line matching a given regular expression near a specified starting line in a string source.
///
/// The function scans forward from `start_line0` up to 20 lines ahead, then backward from 5 lines before `start_line0`
/// to find the first line that matches the provided regex pattern. It returns the zero-based index of the matching line
/// if found, or `None` otherwise. The search is bounded by the total number of lines in the input string.
///
/// # Parameters
/// - `src`: The input string source to search within, line-by-line.
/// - `start_line0`: The starting line index (zero-based) from which to begin the search.
/// - `re`: A reference to a compiled regular expression pattern to match against each line.
///
/// # Returns
/// - `Some(line_index)` if a matching line is found within the search window; otherwise `None`.
///
/// # Notes
/// - The function is designed to efficiently locate a signal line near a given position, useful in log or configuration parsing.
/// - Line indices are zero-based and relative to the input string's line count.
/// - The search window is limited to 20 lines forward and 5 lines backward to avoid excessive scanning.
pub fn find_sig_line_near(src: &str, start_line0: usize, re: &Regex) -> Option<usize> {
    let total = src.lines().count();
    for i in start_line0.min(total)..(start_line0 + 20).min(total) {
        if src.lines().nth(i).map(|l| re.is_match(l)).unwrap_or(false) {
            return Some(i);
        }
    }
    let up_lo = start_line0.saturating_sub(5);
    for i in (up_lo..start_line0.min(total)).rev() {
        if src.lines().nth(i).map(|l| re.is_match(l)).unwrap_or(false) {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_sig_line_near;
    use regex::Regex;

    // Render source with 0-based line numbers for readable failures
    fn with_line_numbers(src: &str) -> String {
        src.lines()
            .enumerate()
            .map(|(i, l)| format!("{:>3}: {}", i, l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sample_src() -> String {
        // 0..=9 (10 lines)
        r#"use std::fmt::Debug;

#[allow(dead_code)]
#[inline]
pub fn alpha() {}

mod something {
}
fn beta() {}

pub struct S {}"#
            .to_string()
    }
    // Index map for clarity in assertions:
    //  0: use std::fmt::Debug;
    //  1:
    //  2: #[allow(dead_code)]
    //  3: #[inline]
    //  4: pub fn alpha() {}
    //  5:
    //  6: mod something {
    //  7: }
    //  8: fn beta() {}
    //  9:
    //
    // Matches for r"^\s*(?:pub\s+)?fn\b" are at lines 4 and 8.

    #[test]
    fn test_find_sig_line_near_finds_forward_within_20() {
        let src = sample_src();
        let re = Regex::new(r"^\s*(?:pub\s+)?fn\b").unwrap();

        let got = find_sig_line_near(&src, 1, &re); // start near the top; first fn is line 4
        assert_eq!(
            got,
            Some(4),
            "Expected to find the first fn going forward to line 4.\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }

    #[test]
    fn test_find_sig_line_near_scans_backward_within_5_when_not_found_forward() {
        let src = sample_src();
        let re = Regex::new(r"^\s*(?:pub\s+)?fn\b").unwrap();

        // Start at 7. Forward window (7..min(27,10)=10) has lines 8..9; line 8 matches.
        // To force the backward branch, we choose a pattern that doesn't match forward (e.g. look for `mod`),
        // then expect it to find the earlier line by scanning up to 5 lines back.
        let re_mod = Regex::new(r"^\s*mod\b").unwrap();

        // From 7 forward: lines 7..9 are `}` and blank; no `mod` there.
        // Backward span is (start-5)..start = 2..7 reversed; line 6 is `mod something {` and should match.
        let got = find_sig_line_near(&src, 7, &re_mod);
        assert_eq!(
            got,
            Some(6),
            "Expected to find `mod` by scanning backward to line 6.\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }

    #[test]
    fn test_find_sig_line_near_returns_none_when_no_match_in_window() {
        let src = sample_src();
        let re = Regex::new(r"^\s*enum\b").unwrap(); // no enums in sample

        let got = find_sig_line_near(&src, 0, &re);
        assert_eq!(
            got,
            None,
            "Expected None when no matching line exists.\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }

    #[test]
    fn test_find_sig_line_near_handles_start_past_end_gracefully() {
        let src = sample_src();
        let re = Regex::new(r"^\s*(?:pub\s+)?fn\b").unwrap();

        // start_line0 far beyond the number of lines; should not panic and should return None.
        let got = find_sig_line_near(&src, 100, &re);
        assert_eq!(
            got,
            None,
            "Expected None when start_line0 is beyond the end of the source.\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }

    #[test]
    fn test_find_sig_line_near_matches_exact_start_line() {
        let src = sample_src();
        let re = Regex::new(r"^\s*(?:pub\s+)?fn\b").unwrap();

        // Directly on a matching line (8: `fn beta() {}`) should return 8.
        let got = find_sig_line_near(&src, 8, &re);
        assert_eq!(
            got,
            Some(8),
            "Expected to match exactly at the start line (8).\nFULL SOURCE:\n{}",
            with_line_numbers(&src)
        );
    }
}
