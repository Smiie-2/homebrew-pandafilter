use super::Handler;

/// Handler for golangci-lint — Go's meta-linter.
/// Groups diagnostics by file; collapses INFO/WARN metadata; shows error count.
pub struct GolangCiLintHandler;

impl Handler for GolangCiLintHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        filter_lint(output)
    }
}

pub fn filter_lint(output: &str) -> String {
    // golangci-lint output format:
    //   src/handler.go:42:9: ineffectual assignment (ineffassign)
    //   src/main.go:15:2: S1000: use plain channel (gosimple)
    // Also has INFO/WARN/ERR prefix lines from the runner itself.

    let mut diagnostics: Vec<String> = Vec::new();
    let mut linter_errors: Vec<String> = Vec::new();
    let mut total = 0usize;
    let clean;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Skip INFO/DEBUG runner lines
        if t.starts_with("INFO") || t.starts_with("DEBU") {
            continue;
        }
        // WARN lines from golangci-lint (configuration warnings etc.)
        if t.starts_with("WARN") {
            if linter_errors.len() < 3 {
                linter_errors.push(t.trim_start_matches("WARN").trim().to_string());
            }
            continue;
        }
        // ERR lines
        if t.starts_with("ERR") || t.starts_with("level=error") {
            linter_errors.push(t.to_string());
            continue;
        }
        // Diagnostic lines: "path/file.go:line:col: message (linter)"
        // Must contain at least one colon and not be a header line
        if looks_like_diagnostic(t) {
            total += 1;
            if total <= 40 {
                diagnostics.push(t.to_string());
            }
            continue;
        }
        // "Run 'golangci-lint ..." hint lines — drop
        if t.starts_with("Run `") || t.starts_with("Run '") {
            continue;
        }
    }

    clean = diagnostics.is_empty() && linter_errors.is_empty();

    if clean {
        return "No issues found.".to_string();
    }

    let mut out: Vec<String> = Vec::new();

    // Group by file for readability
    let grouped = group_by_file(&diagnostics);
    for (file, issues) in &grouped {
        out.push(file.clone());
        for issue in issues {
            out.push(format!("  {}", issue));
        }
    }

    if total > 40 {
        out.push(format!("[+{} more issues]", total - 40));
    }

    out.push(format!("[{} issue(s) found]", total));

    for e in &linter_errors {
        out.push(format!("warn: {}", e));
    }

    out.join("\n")
}

fn looks_like_diagnostic(line: &str) -> bool {
    // "src/foo.go:12:5: some message (linter-name)"
    // Must have at least two colons and the first part should look like a file path
    let parts: Vec<&str> = line.splitn(3, ':').collect();
    if parts.len() < 3 {
        return false;
    }
    let file_part = parts[0];
    // File path: must contain .go or look like a path
    (file_part.ends_with(".go") || file_part.contains('/') || file_part.contains('\\'))
        && parts[1].trim().parse::<u32>().is_ok()
}

fn group_by_file(lines: &[String]) -> Vec<(String, Vec<String>)> {
    let mut map: Vec<(String, Vec<String>)> = Vec::new();

    for line in lines {
        let file = extract_file(line);
        // Remove the file prefix from the issue line for display
        let issue = line
            .splitn(2, ':')
            .nth(1)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| line.clone());

        if let Some(entry) = map.iter_mut().find(|(f, _)| f == &file) {
            entry.1.push(issue);
        } else {
            map.push((file, vec![issue]));
        }
    }

    map
}

fn extract_file(line: &str) -> String {
    line.splitn(2, ':').next().unwrap_or(line).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec![] }

    #[test]
    fn clean_run_returns_no_issues() {
        let output = "\
INFO [config] Config search paths: [/home/user]
INFO [loader] Go packages loading in PACKAGES mode with GOFLAGS=
";
        let result = GolangCiLintHandler.filter(output, &args());
        assert!(result.contains("No issues") || result == "No issues found.");
    }

    #[test]
    fn diagnostics_grouped_by_file() {
        let output = "\
src/handler.go:42:9: ineffectual assignment to err (ineffassign)
src/handler.go:55:3: error return value not checked (errcheck)
src/main.go:15:2: S1000: use plain channel send or receive (gosimple)
";
        let result = GolangCiLintHandler.filter(output, &args());
        assert!(result.contains("src/handler.go"));
        assert!(result.contains("src/main.go"));
        assert!(result.contains("3 issue(s)") || result.contains("issue(s)"));
    }

    #[test]
    fn info_lines_dropped() {
        let output = "\
INFO [runner] Starting linters...
INFO [runner] Running 10 linters
src/foo.go:1:1: unused variable (deadcode)
";
        let result = GolangCiLintHandler.filter(output, &args());
        assert!(!result.contains("INFO"));
        assert!(result.contains("foo.go") || result.contains("issue"));
    }

    #[test]
    fn looks_like_diagnostic_works() {
        assert!(looks_like_diagnostic("src/handler.go:42:9: some message (linter)"));
        assert!(!looks_like_diagnostic("INFO [runner] some info"));
        assert!(!looks_like_diagnostic("WARN deprecated config"));
    }
}
