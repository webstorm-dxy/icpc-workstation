use chrono::{SecondsFormat, Utc};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("tag regex"))
}

fn space_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[ \t\u{00a0}]+").expect("space regex"))
}

fn blank_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\n{3,}").expect("blank regex"))
}

/// Best-effort HTML to readable text/markdown. Deliberately conservative:
/// we keep the content, drop the markup, preserve code blocks and math
/// delimiters, and never execute anything.
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for (from, to) in [
        ("<br />", "\n"),
        ("<br/>", "\n"),
        ("<br>", "\n"),
        ("</p>", "\n\n"),
        ("</div>", "\n"),
        ("</section>", "\n\n"),
        ("</li>", "\n"),
        ("<li>", "\n- "),
        ("</tr>", "\n"),
        ("</h1>", "\n\n"),
        ("</h2>", "\n\n"),
        ("</h3>", "\n\n"),
        ("</h4>", "\n\n"),
        ("</pre>", "\n\n"),
        ("<pre>", "\n```\n"),
    ] {
        s = s.replace(from, to);
    }
    s = tag_re().replace_all(&s, "").to_string();
    s = decode_entities(&s);
    s = s.replace("\r\n", "\n").replace('\r', "\n");
    // Normalize common LaTeX delimiters to the form remark-math understands.
    s = s.replace("\\(", "$").replace("\\)", "$");
    s = s.replace("\\[", "$$").replace("\\]", "$$");
    let lines: Vec<String> = s
        .lines()
        .map(|line| space_re().replace_all(line, " ").trim_end().to_string())
        .collect();
    s = lines.join("\n");
    s = blank_re().replace_all(&s, "\n\n").to_string();
    s.trim().to_string()
}

pub fn decode_entities(input: &str) -> String {
    let mut s = input
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&hellip;", "…")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&le;", "≤")
        .replace("&ge;", "≥")
        .replace("&ne;", "≠");
    let numeric = Regex::new(r"&#(x?[0-9a-fA-F]+);").expect("entity regex");
    s = numeric
        .replace_all(&s, |caps: &regex::Captures| {
            let raw = &caps[1];
            let code = if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                raw.parse::<u32>().ok()
            };
            code.and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| caps[0].to_string())
        })
        .to_string();
    s
}

#[derive(Debug, Clone, Copy)]
pub struct NormalizeOptions {
    pub trim_line_endings: bool,
    pub trim_trailing_blank_lines: bool,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        Self {
            trim_line_endings: true,
            trim_trailing_blank_lines: true,
        }
    }
}

pub fn normalize_output(input: &str, options: NormalizeOptions) -> String {
    let mut s = input.replace("\r\n", "\n").replace('\r', "\n");
    if options.trim_line_endings {
        s = s
            .lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
    }
    if options.trim_trailing_blank_lines {
        s = s.trim_end_matches('\n').to_string();
    }
    s
}

pub fn norm_verdict(raw: &str) -> String {
    let key = raw.trim().to_ascii_uppercase().replace([' ', '-'], "_");
    match key.as_str() {
        "OK" | "AC" | "ACCEPTED" | "PASSED" => "AC".into(),
        "WRONG_ANSWER" | "WA" | "WRONG" => "WA".into(),
        "TIME_LIMIT_EXCEEDED" | "TLE" | "TIME_LIMIT" => "TLE".into(),
        "MEMORY_LIMIT_EXCEEDED" | "MLE" | "MEMORY_LIMIT" => "MLE".into(),
        "RUNTIME_ERROR" | "RE" | "CRASHED" | "SECURITY_VIOLATED" => "RE".into(),
        "COMPILATION_ERROR" | "CE" | "COMPILE_ERROR" => "CE".into(),
        "PRESENTATION_ERROR" | "PE" => "PE".into(),
        "OUTPUT_LIMIT_EXCEEDED" | "OLE" => "OLE".into(),
        "IDLENESS_LIMIT_EXCEEDED" | "ILE" => "ILE".into(),
        "PARTIAL" | "PARTIALLY_ACCEPTED" => "PARTIAL".into(),
        "JUDGING" | "TESTING" | "WJ" | "WAITING" => "JUDGING".into(),
        "SKIPPED" => "SKIPPED".into(),
        "CHALLENGED" => "WA".into(),
        "FAILED" | "REJECTED" => "FAILED".into(),
        other => other.to_string(),
    }
}

pub fn verdict_from_cf(raw: &str) -> String {
    norm_verdict(raw)
}

pub fn verdict_from_atcoder(raw: &str) -> String {
    norm_verdict(raw)
}

pub fn verdict_from_luogu(status: i64) -> String {
    match status {
        12 => "AC",
        6 => "WA",
        5 => "TLE",
        4 => "MLE",
        3 => "OLE",
        7 => "RE",
        2 => "CE",
        1 => "JUDGING",
        0 => "JUDGING",
        _ => "UNKNOWN",
    }
    .to_string()
}

pub fn is_ac(verdict: &str) -> bool {
    verdict == "AC"
}

pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_chars).collect();
    out.push_str("\n…（内容过长已截断）");
    out
}

pub fn strip_code_fence(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let without_start = trimmed
        .strip_prefix("```")
        .unwrap_or(trimmed)
        .trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ' ')
        .trim_start_matches('\n');
    if let Some(pos) = without_start.rfind("```") {
        without_start[..pos].trim().to_string()
    } else {
        without_start.trim().to_string()
    }
}

pub fn platform_label(platform: &str) -> &'static str {
    match platform {
        "codeforces" => "Codeforces",
        "atcoder" => "AtCoder",
        "luogu" => "洛谷",
        _ => "Unknown",
    }
}

pub fn parse_json_array(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_whitespace_and_newlines() {
        let opts = NormalizeOptions::default();
        assert_eq!(normalize_output("a  \r\nb\r\n\r\n", opts), "a\nb");
    }

    #[test]
    fn converts_verdicts() {
        assert_eq!(norm_verdict("OK"), "AC");
        assert_eq!(norm_verdict("WRONG_ANSWER"), "WA");
        assert_eq!(verdict_from_luogu(12), "AC");
        assert_eq!(verdict_from_luogu(6), "WA");
    }

    #[test]
    fn strips_html_and_keeps_code() {
        let html = "<p>Hello &amp; <b>world</b></p><pre>1 2 3</pre>";
        let text = html_to_text(html);
        assert!(text.contains("Hello & world"));
        assert!(text.contains("1 2 3"));
        assert!(!text.contains("<b>"));
    }

    #[test]
    fn strips_json_fences() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
    }
}
