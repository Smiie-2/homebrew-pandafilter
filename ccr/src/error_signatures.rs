//! Error-loop detection: structural diff of error sets across retries.
//!
//! When Claude re-runs a command (cargo build, tsc, pytest) and gets similar
//! errors, this module compares error sets structurally (by code + file + message)
//! and produces a compact diff: fixed / new / unchanged — instead of re-emitting
//! the full output every time.

use once_cell::sync::OnceCell;
use regex::Regex;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct ErrorSignature {
    pub code: Option<String>,
    pub file: Option<String>,
    pub line_no: Option<usize>,
    pub message: String,
    pub severity: ErrorSeverity,
    /// Raw display lines (error line + optional location line)
    pub raw_lines: Vec<String>,
}

impl ErrorSignature {
    /// Stable key for structural comparison: code | file | normalised-message.
    /// Line numbers are excluded so "same error, different column" still matches.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.code.as_deref().unwrap_or(""),
            self.file.as_deref().unwrap_or(""),
            self.message.trim().to_lowercase()
        )
    }

    pub fn display(&self) -> String {
        if self.raw_lines.is_empty() {
            format!(
                "{}: {}",
                self.code.as_deref().unwrap_or("error"),
                self.message
            )
        } else {
            self.raw_lines.join("\n")
        }
    }
}

#[derive(Debug, Default)]
pub struct ErrorSet {
    pub signatures: Vec<ErrorSignature>,
}

impl ErrorSet {
    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Parse an error set from command output.
    /// Handles Rust, TypeScript, Python, Go, and generic error lines.
    pub fn from_output(output: &str) -> Self {
        let lines: Vec<&str> = output.lines().collect();
        let n = lines.len();
        let mut signatures: Vec<ErrorSignature> = Vec::new();
        let mut i = 0;

        while i < n {
            let line = lines[i];
            if let Some(sig) = parse_rust_error(line, &lines, i) {
                let skip = sig.raw_lines.len().max(1);
                signatures.push(sig);
                i += skip;
                continue;
            }
            if let Some(sig) = parse_ts_error(line) {
                signatures.push(sig);
                i += 1;
                continue;
            }
            if let Some(sig) = parse_python_error(line, &lines, i) {
                signatures.push(sig);
                i += 1;
                continue;
            }
            if let Some(sig) = parse_go_error(line) {
                signatures.push(sig);
                i += 1;
                continue;
            }
            if let Some(sig) = parse_generic_error(line) {
                signatures.push(sig);
            }
            i += 1;
        }

        // Deduplicate by key (same error can appear multiple times in noisy output)
        let mut seen = std::collections::HashSet::new();
        signatures.retain(|s| seen.insert(s.key()));
        ErrorSet { signatures }
    }

    /// Structural diff: categorise current errors vs a prior set.
    pub fn diff(&self, prior: &ErrorSet) -> ErrorDiff {
        let current_keys: std::collections::HashSet<String> =
            self.signatures.iter().map(|s| s.key()).collect();
        let prior_keys: std::collections::HashSet<String> =
            prior.signatures.iter().map(|s| s.key()).collect();

        let fixed: Vec<ErrorSignature> = prior
            .signatures
            .iter()
            .filter(|s| !current_keys.contains(&s.key()))
            .cloned()
            .collect();
        let new_errors: Vec<ErrorSignature> = self
            .signatures
            .iter()
            .filter(|s| !prior_keys.contains(&s.key()))
            .cloned()
            .collect();
        let unchanged: Vec<ErrorSignature> = self
            .signatures
            .iter()
            .filter(|s| prior_keys.contains(&s.key()))
            .cloned()
            .collect();

        ErrorDiff { fixed, new_errors, unchanged }
    }

    /// Compact serialisation for storage in `SessionEntry.error_signatures`.
    pub fn to_storage(&self) -> String {
        self.signatures
            .iter()
            .map(|s| s.key())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Deserialise from storage string. Raw lines are unavailable after round-trip.
    pub fn from_storage(s: &str) -> Self {
        let signatures = s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|key| {
                let parts: Vec<&str> = key.splitn(3, '|').collect();
                ErrorSignature {
                    code: parts.first().filter(|s| !s.is_empty()).map(|s| s.to_string()),
                    file: parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                    line_no: None,
                    message: parts.get(2).copied().unwrap_or("").to_string(),
                    severity: ErrorSeverity::Error,
                    raw_lines: Vec::new(),
                }
            })
            .collect();
        ErrorSet { signatures }
    }
}

#[derive(Debug)]
pub struct ErrorDiff {
    pub fixed: Vec<ErrorSignature>,
    pub new_errors: Vec<ErrorSignature>,
    pub unchanged: Vec<ErrorSignature>,
}

impl ErrorDiff {
    /// True when there is overlap with prior errors (i.e. an actual loop).
    pub fn has_loop(&self) -> bool {
        !self.unchanged.is_empty()
    }
}

// ── Language parsers ──────────────────────────────────────────────────────────

fn parse_rust_error(line: &str, lines: &[&str], i: usize) -> Option<ErrorSignature> {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^(error|warning)\[([EW]\d+)\]:\s+(.+)$").unwrap()
    });

    let caps = re.captures(line)?;
    let severity = if caps[1].starts_with('w') {
        ErrorSeverity::Warning
    } else {
        ErrorSeverity::Error
    };
    let code = caps[2].to_string();
    let message = caps[3].to_string();
    let mut raw_lines = vec![line.to_string()];

    let (file, line_no) = if let Some(next) = lines.get(i + 1) {
        if next.trim_start().starts_with("-->") {
            raw_lines.push(next.to_string());
            parse_rust_location(next)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Some(ErrorSignature { code: Some(code), file, line_no, message, severity, raw_lines })
}

fn parse_rust_location(line: &str) -> (Option<String>, Option<usize>) {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re = RE.get_or_init(|| Regex::new(r"-->\s+([^:]+):(\d+):\d+").unwrap());
    re.captures(line.trim())
        .map(|c| (Some(c[1].to_string()), c[2].parse().ok()))
        .unwrap_or((None, None))
}

fn parse_ts_error(line: &str) -> Option<ErrorSignature> {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^([^(]+\.tsx?)\((\d+),\d+\): (error|warning) (TS\d+):\s+(.+)$").unwrap()
    });

    let caps = re.captures(line)?;
    let severity = if caps[3].starts_with('w') {
        ErrorSeverity::Warning
    } else {
        ErrorSeverity::Error
    };
    Some(ErrorSignature {
        code: Some(caps[4].to_string()),
        file: Some(caps[1].trim().to_string()),
        line_no: caps[2].parse().ok(),
        message: caps[5].to_string(),
        severity,
        raw_lines: vec![line.to_string()],
    })
}

fn parse_python_error(line: &str, lines: &[&str], i: usize) -> Option<ErrorSignature> {
    static RE_FILE: OnceCell<Regex> = OnceCell::new();
    static RE_ERR: OnceCell<Regex> = OnceCell::new();
    let re_file = RE_FILE
        .get_or_init(|| Regex::new(r#"File "([^"]+)", line (\d+)"#).unwrap());
    let re_err = RE_ERR.get_or_init(|| {
        Regex::new(r"^([A-Za-z][A-Za-z0-9_]*(?:Error|Exception|Warning)):\s+(.+)$").unwrap()
    });

    let file_caps = re_file.captures(line)?;
    let file = file_caps[1].to_string();
    let line_no: Option<usize> = file_caps[2].parse().ok();

    for j in (i + 1)..std::cmp::min(i + 5, lines.len()) {
        if let Some(err_caps) = re_err.captures(lines[j]) {
            return Some(ErrorSignature {
                code: Some(err_caps[1].to_string()),
                file: Some(file),
                line_no,
                message: err_caps[2].to_string(),
                severity: ErrorSeverity::Error,
                raw_lines: vec![line.to_string(), lines[j].to_string()],
            });
        }
    }
    None
}

fn parse_go_error(line: &str) -> Option<ErrorSignature> {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re =
        RE.get_or_init(|| Regex::new(r"^([^:]+\.go):(\d+):\d+:\s+(.{5,})$").unwrap());

    let caps = re.captures(line)?;
    let message = caps[3].to_string();
    let severity = if message.to_lowercase().contains("warning") {
        ErrorSeverity::Warning
    } else {
        ErrorSeverity::Error
    };
    Some(ErrorSignature {
        code: None,
        file: Some(caps[1].to_string()),
        line_no: caps[2].parse().ok(),
        message,
        severity,
        raw_lines: vec![line.to_string()],
    })
}

fn parse_generic_error(line: &str) -> Option<ErrorSignature> {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:error|FAILED|fatal|FATAL):\s+(.{10,})$").unwrap()
    });

    let trimmed = line.trim();
    // Skip summary/aggregation lines — these vary by count and produce false
    // "unchanged" matches across runs where errors actually changed.
    if trimmed.to_lowercase().contains("aborting due to")
        || trimmed.to_lowercase().contains("previous error")
        || trimmed.to_lowercase().contains("could not compile")
    {
        return None;
    }

    let caps = re.captures(trimmed)?;
    Some(ErrorSignature {
        code: None,
        file: None,
        line_no: None,
        message: caps[1].trim().to_string(),
        severity: ErrorSeverity::Error,
        raw_lines: vec![line.to_string()],
    })
}

// ── Top-level detection ───────────────────────────────────────────────────────

/// Check if the current output is part of an error loop.
/// Returns `Some(structural_diff_output)` when a loop is detected.
/// Returns `None` to fall through to C3 unchanged.
///
/// Safe to call unconditionally — returns `None` in all non-loop cases:
/// - No errors in current output
/// - No prior run with errors for this command
/// - All current errors are new (first encounter)
pub fn apply_error_loop_detection(
    output: &str,
    cmd_key: &str,
    session: &crate::session::SessionState,
) -> Option<String> {
    let current = ErrorSet::from_output(output);
    if current.is_empty() {
        return None;
    }

    let (prior_turn, prior_storage) = session.find_error_loop(cmd_key)?;
    let prior = ErrorSet::from_storage(prior_storage);
    if prior.is_empty() {
        return None;
    }

    let diff = current.diff(&prior);
    if !diff.has_loop() {
        return None; // All errors are new — not a loop; C3 handles it
    }

    let current_turn = session.total_turns + 1;
    Some(build_diff_output(&diff, prior_turn, current_turn, cmd_key))
}

fn build_diff_output(
    diff: &ErrorDiff,
    prior_turn: usize,
    current_turn: usize,
    cmd_key: &str,
) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "[Error loop: turn {} → turn {} | {}]\n",
        prior_turn, current_turn, cmd_key
    ));

    // Summary
    let mut parts = Vec::new();
    if !diff.fixed.is_empty() {
        parts.push(format!(
            "[{} error{} fixed]",
            diff.fixed.len(),
            if diff.fixed.len() == 1 { "" } else { "s" }
        ));
    }
    if !diff.new_errors.is_empty() {
        parts.push(format!(
            "[{} new error{}]",
            diff.new_errors.len(),
            if diff.new_errors.len() == 1 { "" } else { "s" }
        ));
    }
    if !diff.unchanged.is_empty() {
        parts.push(format!("[{} unchanged]", diff.unchanged.len()));
    }
    if !parts.is_empty() {
        out.push_str(&parts.join(" "));
        out.push('\n');
    }

    // Fixed (brief — just the message)
    if !diff.fixed.is_empty() {
        out.push_str("\n── Fixed ──\n");
        for sig in &diff.fixed {
            out.push_str(&format!(
                "✓ {}\n",
                sig.message.chars().take(100).collect::<String>()
            ));
        }
    }

    // New errors (full display)
    if !diff.new_errors.is_empty() {
        out.push_str("\n── New errors ──\n");
        for sig in &diff.new_errors {
            out.push_str(&sig.display());
            out.push('\n');
        }
    }

    // Unchanged errors — collapse into zoom block when zoom is active
    if !diff.unchanged.is_empty() {
        let n = diff.unchanged.len();
        let unchanged_lines: Vec<String> = diff
            .unchanged
            .iter()
            .flat_map(|s| s.raw_lines.iter().cloned())
            .collect();

        if panda_core::zoom::is_enabled() && !unchanged_lines.is_empty() {
            let zi_id = panda_core::zoom::register(unchanged_lines);
            out.push_str(&format!(
                "\n── Unchanged errors (collapsed) ──\n[{} error{} unchanged from turn {} — panda expand {}]\n",
                n,
                if n == 1 { "" } else { "s" },
                prior_turn,
                zi_id
            ));
        } else {
            out.push_str("\n── Unchanged errors ──\n");
            for sig in &diff.unchanged {
                out.push_str(&sig.display());
                out.push('\n');
            }
        }
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rust_error_extracts_code_and_message() {
        let output = "error[E0308]: mismatched types\n  --> src/main.rs:42:5";
        let set = ErrorSet::from_output(output);
        assert_eq!(set.len(), 1);
        let sig = &set.signatures[0];
        assert_eq!(sig.code.as_deref(), Some("E0308"));
        assert_eq!(sig.file.as_deref(), Some("src/main.rs"));
        assert_eq!(sig.line_no, Some(42));
    }

    #[test]
    fn parse_ts_error_extracts_code_and_file() {
        let output = "src/index.ts(10,5): error TS2345: Argument of type 'string' is not assignable";
        let set = ErrorSet::from_output(output);
        assert_eq!(set.len(), 1);
        let sig = &set.signatures[0];
        assert_eq!(sig.code.as_deref(), Some("TS2345"));
        assert_eq!(sig.file.as_deref(), Some("src/index.ts"));
    }

    #[test]
    fn parse_go_error_extracts_file_and_line() {
        let output = "cmd/main.go:42:10: undefined: foo";
        let set = ErrorSet::from_output(output);
        assert_eq!(set.len(), 1);
        let sig = &set.signatures[0];
        assert_eq!(sig.file.as_deref(), Some("cmd/main.go"));
        assert_eq!(sig.line_no, Some(42));
    }

    #[test]
    fn diff_detects_fixed_new_unchanged() {
        let prior_output =
            "error[E0308]: mismatched types\n  --> src/main.rs:10:5\nerror[E0277]: trait not implemented\n  --> src/lib.rs:20:1";
        let current_output =
            "error[E0308]: mismatched types\n  --> src/main.rs:10:5\nerror[E0061]: wrong number of arguments\n  --> src/main.rs:30:5";

        let prior = ErrorSet::from_output(prior_output);
        let current = ErrorSet::from_output(current_output);
        let diff = current.diff(&prior);

        assert_eq!(diff.fixed.len(), 1); // E0277 fixed
        assert_eq!(diff.new_errors.len(), 1); // E0061 new
        assert_eq!(diff.unchanged.len(), 1); // E0308 unchanged
        assert!(diff.has_loop());
    }

    #[test]
    fn all_new_errors_is_not_a_loop() {
        let prior_output = "error[E0308]: mismatched types\n  --> src/main.rs:10:5";
        let current_output = "error[E0061]: wrong number of arguments\n  --> src/main.rs:30:5";

        let prior = ErrorSet::from_output(prior_output);
        let current = ErrorSet::from_output(current_output);
        let diff = current.diff(&prior);

        assert!(!diff.has_loop());
    }

    #[test]
    fn empty_output_produces_empty_set() {
        assert!(ErrorSet::from_output("").is_empty());
        assert!(ErrorSet::from_output("no errors here, all good!").is_empty());
    }

    #[test]
    fn storage_roundtrip_preserves_keys() {
        let output = "error[E0308]: mismatched types\n  --> src/main.rs:10:5\nerror[E0277]: trait not implemented\n  --> src/lib.rs:20:1";
        let original = ErrorSet::from_output(output);
        let stored = original.to_storage();
        let restored = ErrorSet::from_storage(&stored);
        let orig_keys: Vec<String> = original.signatures.iter().map(|s| s.key()).collect();
        let rest_keys: Vec<String> = restored.signatures.iter().map(|s| s.key()).collect();
        assert_eq!(orig_keys, rest_keys);
    }
}
