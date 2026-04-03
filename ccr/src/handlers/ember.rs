use super::Handler;

pub struct EmberHandler;

impl Handler for EmberHandler {
    fn filter(&self, output: &str, args: &[String]) -> String {
        let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
        match subcmd {
            "build" | "b" => filter_build(output),
            "test" | "t" => filter_test(output),
            "serve" | "s" => filter_serve(output),
            // generate/destroy output is already short — passthrough
            _ => output.to_string(),
        }
    }
}

fn filter_build(output: &str) -> String {
    let mut errors: Vec<&str> = Vec::new();
    let mut summary: Option<&str> = None;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Final summary lines — covers "Build successful", "Built project successfully", "Build failed"
        if (t.starts_with("Build") || t.starts_with("Built"))
            && (t.contains("successful") || t.contains("failed"))
        {
            summary = Some(line);
            continue;
        }
        // Error lines: TypeScript errors, template errors, JS/TS file references
        if t.contains("Error:")
            || t.contains("error TS")
            || t.contains(".hbs:")
            || t.contains(".js:")
            || t.contains(".ts:")
            || t.contains(".gjs:")
            || t.contains(".gts:")
        {
            errors.push(line);
        }
        // Everything else (progress lines, fingerprint spam) is dropped
    }

    // Cap at 40 error lines
    let capped: Vec<&str> = errors.iter().copied().take(40).collect();
    let extra = errors.len().saturating_sub(40);

    let mut out: Vec<String> = capped.iter().map(|l| l.to_string()).collect();
    if extra > 0 {
        out.push(format!("[+{} more errors]", extra));
    }
    if let Some(s) = summary {
        out.push(s.to_string());
    }

    if out.is_empty() {
        // No errors and no summary — output was likely just progress noise
        output.to_string()
    } else {
        out.join("\n")
    }
}

fn filter_test(output: &str) -> String {
    let mut failures: Vec<String> = Vec::new();
    let mut summary: Option<String> = None;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // TAP-style passing lines ("ok N - description") — drop
        if t.starts_with("ok ") && !t.starts_with("ok 0") {
            continue;
        }
        // Summary lines: "X passed, Y failed" or "# X tests"
        if (t.contains("passed") || t.contains("failed"))
            && (t.contains(',') || t.starts_with("# ") || t.starts_with("1.."))
        {
            summary = Some(line.to_string());
            continue;
        }
        // Failing test lines
        if t.starts_with("not ok")
            || t.contains("FAILED")
            || t.contains("AssertionError")
            || (t.contains("Error:") && !t.contains("error TS"))
        {
            failures.push(line.to_string());
        }
    }

    if failures.is_empty() {
        if let Some(s) = summary {
            return s;
        }
        return "[all tests passed]".to_string();
    }

    let mut out = failures;
    if let Some(s) = summary {
        out.push(s);
    }
    out.join("\n")
}

fn filter_serve(output: &str) -> String {
    // Keep only the "Serving on http://localhost:XXXX" line
    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("Serving on") || t.contains("http://localhost") {
            return line.to_string();
        }
    }
    // Fallback: passthrough if no serving line found
    output.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args(subcmd: &str) -> Vec<String> {
        vec!["ember".to_string(), subcmd.to_string()]
    }

    // ── filter_build ──────────────────────────────────────────────────────────

    #[test]
    fn build_keeps_error_lines_and_summary() {
        let output = "Building...\n\
                      app/templates/index.hbs:5:3: Error: Unexpected token\n\
                      app/components/foo.ts:12:1: error TS2345: Argument of type 'string'\n\
                      Built project successfully (1234ms)";
        let result = EmberHandler.filter(output, &args("build"));
        assert!(result.contains("Error: Unexpected token"), "should keep .hbs error");
        assert!(result.contains("error TS2345"), "should keep TS error");
        assert!(!result.contains("Building..."), "should drop progress noise");
        assert!(result.contains("Built project successfully"), "should keep summary");
    }

    #[test]
    fn build_failed_summary_kept() {
        let output = "Building...\nsome noise\nBuild failed.";
        let result = EmberHandler.filter(output, &args("build"));
        assert!(result.contains("Build failed"));
    }

    #[test]
    fn build_caps_at_40_errors() {
        let many_errors: String = (0..50)
            .map(|i| format!("app/components/x.js:{}:1: Error: msg {}\n", i, i))
            .collect();
        let result = EmberHandler.filter(&many_errors, &args("build"));
        assert!(
            result.contains("[+10 more errors]"),
            "should cap at 40 and show overflow: got {:?}",
            result
        );
    }

    #[test]
    fn build_passthrough_when_no_errors_or_summary() {
        let output = "Building...\nSome random non-error progress line";
        let result = EmberHandler.filter(output, &args("build"));
        assert_eq!(result, output);
    }

    // ── filter_test ───────────────────────────────────────────────────────────

    #[test]
    fn test_keeps_failing_tests_and_summary() {
        let output = "ok 1 - MyApp: foo passes\n\
                      not ok 2 - MyApp: bar fails\n\
                      # Error: expected true got false\n\
                      # 1 passed, 1 failed";
        let result = EmberHandler.filter(output, &args("test"));
        assert!(result.contains("not ok 2"), "should keep failing test");
        assert!(!result.contains("ok 1 - MyApp: foo"), "should drop passing test");
        assert!(result.contains("1 passed, 1 failed"), "should keep summary");
    }

    #[test]
    fn test_all_pass_returns_summary() {
        let output = "ok 1 - MyApp: foo\nok 2 - MyApp: bar\n# 2 passed, 0 failed";
        let result = EmberHandler.filter(output, &args("test"));
        assert!(
            result.contains("passed"),
            "should include pass summary: got {:?}",
            result
        );
    }

    #[test]
    fn test_no_output_returns_all_passed() {
        let output = "ok 1 - test\nok 2 - test2";
        let result = EmberHandler.filter(output, &args("test"));
        assert!(
            result.contains("passed") || result == "[all tests passed]",
            "should indicate all passed: got {:?}",
            result
        );
    }

    // ── filter_serve ──────────────────────────────────────────────────────────

    #[test]
    fn serve_keeps_only_localhost_line() {
        let output = "Build successful (1234ms)\n\
                      Serving on http://localhost:4200\n\
                      Watching for changes...";
        let result = EmberHandler.filter(output, &args("serve"));
        assert_eq!(result.trim(), "Serving on http://localhost:4200");
    }

    #[test]
    fn serve_passthrough_when_no_serving_line() {
        let output = "Starting server...\nInitializing...";
        let result = EmberHandler.filter(output, &args("serve"));
        assert_eq!(result, output);
    }

    // ── generate / destroy passthrough ────────────────────────────────────────

    #[test]
    fn generate_passthrough() {
        let output = "installing component\n  create app/components/my-widget.js\n  create tests/integration/components/my-widget-test.js";
        let result = EmberHandler.filter(output, &args("generate"));
        assert_eq!(result, output);
    }

    #[test]
    fn destroy_passthrough() {
        let output = "removing component\n  remove app/components/my-widget.js";
        let result = EmberHandler.filter(output, &args("destroy"));
        assert_eq!(result, output);
    }

    #[test]
    fn short_alias_b_routes_to_build() {
        let output = "Building...\nBuild failed.";
        let result = EmberHandler.filter(output, &args("b"));
        assert!(result.contains("Build failed"));
    }

    #[test]
    fn short_alias_s_routes_to_serve() {
        let output = "Build successful\nServing on http://localhost:4200\nWatching...";
        let result = EmberHandler.filter(output, &args("s"));
        assert_eq!(result.trim(), "Serving on http://localhost:4200");
    }
}
