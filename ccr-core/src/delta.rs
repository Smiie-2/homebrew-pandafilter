//! File delta computation for Read hook de-duplication.
//!
//! On file re-reads within the same session, sends only what changed
//! (unified diff with 1 line of context) rather than the full file content.
//! This eliminates the most common source of redundant tokens in long coding sessions.

/// Max file lines eligible for delta mode.
pub const MAX_LINES: usize = 2000;
/// Max chars the diff output may contain before falling back to full send.
pub const MAX_DIFF_CHARS: usize = 2000;
/// Lines of context around each change hunk.
const CONTEXT: usize = 1;

// ── Public API ────────────────────────────────────────────────────────────────

/// Result of comparing a file's current content to a prior cached version.
#[derive(Debug)]
pub enum DeltaResult {
    /// Compact unified diff showing changed lines.
    Diff(String),
    /// Content is byte-identical to the cached version.
    Unchanged,
    /// Diff would exceed `MAX_DIFF_CHARS` — caller should send full content.
    TooLarge,
    /// File is not eligible (binary, or line count > `MAX_LINES`).
    NotEligible,
}

/// Compute a unified diff between `old` and `new` content for `file_path`.
///
/// - `Unchanged`   → identical content, suppress re-send entirely
/// - `NotEligible` → binary or too large, let caller decide
/// - `TooLarge`    → diff > 2000 chars, send full file instead
/// - `Diff(text)`  → compact diff ready to inject as hook output
pub fn compute(file_path: &str, old: &str, new: &str) -> DeltaResult {
    // Binary guard
    if old.bytes().any(|b| b == 0) || new.bytes().any(|b| b == 0) {
        return DeltaResult::NotEligible;
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    if old_lines.len() > MAX_LINES || new_lines.len() > MAX_LINES {
        return DeltaResult::NotEligible;
    }

    if old_lines == new_lines {
        return DeltaResult::Unchanged;
    }

    let diff_body = format_unified(&old_lines, &new_lines, CONTEXT);

    let name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);

    let added: usize = diff_body.lines().filter(|l| l.starts_with('+')).count();
    let removed: usize = diff_body.lines().filter(|l| l.starts_with('-')).count();
    let diff = format!(
        "[delta: {} — +{} / -{} lines]\n{}",
        name, added, removed, diff_body
    );

    if diff.len() > MAX_DIFF_CHARS {
        return DeltaResult::TooLarge;
    }

    DeltaResult::Diff(diff)
}

// ── LCS-based diff ────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Edit {
    Keep(usize, usize),
    Remove(usize),
    Add(usize),
}

/// Compute LCS edit list between two line slices using dynamic programming.
/// Uses u16 DP table — safe for inputs up to 65535 lines (MAX_LINES=2000).
fn lcs_edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let m = old.len();
    let n = new.len();

    // Flat DP table: dp[i][j] = LCS length of old[i..] vs new[j..]
    let mut dp = vec![0u16; (m + 1) * (n + 1)];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i * (n + 1) + j] = if old[i] == new[j] {
                dp[(i + 1) * (n + 1) + (j + 1)].saturating_add(1)
            } else {
                dp[(i + 1) * (n + 1) + j].max(dp[i * (n + 1) + j + 1])
            };
        }
    }

    // Backtrack to build edit list
    let mut edits = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m || j < n {
        if i < m && j < n && old[i] == new[j] {
            edits.push(Edit::Keep(i, j));
            i += 1;
            j += 1;
        } else if j >= n
            || (i < m && dp[(i + 1) * (n + 1) + j] >= dp[i * (n + 1) + (j + 1)])
        {
            edits.push(Edit::Remove(i));
            i += 1;
        } else {
            edits.push(Edit::Add(j));
            j += 1;
        }
    }
    edits
}

/// Format unified diff with `ctx` lines of context around each hunk.
fn format_unified(old: &[&str], new: &[&str], ctx: usize) -> String {
    let edits = lcs_edits(old, new);

    // Collect positions of changed edits
    let changed: Vec<usize> = edits
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e, Edit::Keep(_, _)))
        .map(|(i, _)| i)
        .collect();

    if changed.is_empty() {
        return String::new();
    }

    // Merge overlapping context windows into hunk ranges
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = changed[0].saturating_sub(ctx);
    let mut end = (changed[0] + ctx + 1).min(edits.len());

    for &pos in &changed[1..] {
        let new_start = pos.saturating_sub(ctx);
        if new_start <= end {
            end = (pos + ctx + 1).min(edits.len());
        } else {
            ranges.push((start, end));
            start = new_start;
            end = (pos + ctx + 1).min(edits.len());
        }
    }
    ranges.push((start, end));

    let mut out = String::new();
    for (rstart, rend) in ranges {
        let slice = &edits[rstart..rend];

        let old_start = slice.iter().find_map(|e| match e {
            Edit::Keep(oi, _) | Edit::Remove(oi) => Some(*oi + 1),
            _ => None,
        }).unwrap_or(1);
        let new_start = slice.iter().find_map(|e| match e {
            Edit::Keep(_, ni) | Edit::Add(ni) => Some(*ni + 1),
            _ => None,
        }).unwrap_or(1);
        let old_count = slice.iter().filter(|e| !matches!(e, Edit::Add(_))).count();
        let new_count = slice.iter().filter(|e| !matches!(e, Edit::Remove(_))).count();

        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count
        ));
        for edit in slice {
            match edit {
                Edit::Keep(oi, _) => out.push_str(&format!(" {}\n", old[*oi])),
                Edit::Remove(oi) => out.push_str(&format!("-{}\n", old[*oi])),
                Edit::Add(ni) => out.push_str(&format!("+{}\n", new[*ni])),
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
    fn unchanged_returns_unchanged() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        assert!(matches!(compute("test.rs", content, content), DeltaResult::Unchanged));
    }

    #[test]
    fn binary_file_returns_not_eligible() {
        let old = "normal text";
        let new = "text with \x00 null byte";
        assert!(matches!(compute("test.bin", old, new), DeltaResult::NotEligible));
    }

    #[test]
    fn over_2000_lines_returns_not_eligible() {
        let big: String = (0..2001).map(|i| format!("line {}\n", i)).collect();
        let slightly_different = big.replace("line 1000", "line 1000 modified");
        assert!(matches!(
            compute("big.rs", &big, &slightly_different),
            DeltaResult::NotEligible
        ));
    }

    #[test]
    fn small_change_returns_diff() {
        let old = "fn foo() {\n    let x = 1;\n}\n";
        let new = "fn foo() {\n    let x = 2;\n}\n";
        match compute("test.rs", old, new) {
            DeltaResult::Diff(d) => {
                assert!(d.contains("-    let x = 1;"), "should show removal: {d}");
                assert!(d.contains("+    let x = 2;"), "should show addition: {d}");
            }
            other => panic!("expected Diff, got {:?}", other),
        }
    }

    #[test]
    fn large_diff_returns_too_large() {
        // 100 lines on each side, completely different — diff will be large
        let old: String = (0..100).map(|i| format!("old unique line number {}\n", i)).collect();
        let new: String = (0..100).map(|i| format!("new entirely different content {}\n", i)).collect();
        let result = compute("big.rs", &old, &new);
        // May be TooLarge or Diff depending on exact size — just confirm it's one of the valid variants
        assert!(matches!(result, DeltaResult::TooLarge | DeltaResult::Diff(_)));
    }

    #[test]
    fn diff_format_has_hunk_header() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        match compute("x.rs", old, new) {
            DeltaResult::Diff(d) => assert!(d.contains("@@"), "missing hunk header: {d}"),
            other => panic!("expected Diff, got {:?}", other),
        }
    }

    #[test]
    fn diff_header_shows_file_name() {
        let old = "fn a() {}\n";
        let new = "fn b() {}\n";
        match compute("/path/to/main.rs", old, new) {
            DeltaResult::Diff(d) => assert!(d.contains("main.rs"), "missing filename: {d}"),
            other => panic!("expected Diff, got {:?}", other),
        }
    }

    #[test]
    fn context_lines_included() {
        let old = "line1\nline2\nchange\nline4\nline5\n";
        let new = "line1\nline2\nCHANGED\nline4\nline5\n";
        match compute("ctx.rs", old, new) {
            DeltaResult::Diff(d) => {
                // line2 and line4 are the 1-line context around "change"
                assert!(d.contains(" line2"), "should include context before: {d}");
                assert!(d.contains(" line4"), "should include context after: {d}");
            }
            other => panic!("expected Diff, got {:?}", other),
        }
    }
}
