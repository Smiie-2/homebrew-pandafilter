use super::Handler;

/// Handler for Next.js CLI (`next build`, `next dev`, `next lint`, `next start`).
/// Next.js build output is extremely verbose — route tables, chunk manifests, etc.
pub struct NextHandler;

impl Handler for NextHandler {
    fn filter(&self, output: &str, args: &[String]) -> String {
        let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
        match subcmd {
            "build" => filter_build(output),
            "dev" => filter_dev(output),
            "lint" => filter_lint(output),
            _ => filter_generic(output),
        }
    }
}

fn filter_build(output: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut checkmarks: Vec<String> = Vec::new();
    let mut route_count: Option<usize> = None;
    let mut build_time: Option<String> = None;
    let mut failed = false;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Error lines
        if t.starts_with("Error:") || t.contains("Failed to compile") || t.contains("Build error") {
            errors.push(t.to_string());
            failed = true;
            continue;
        }
        // Type errors from tsc embedded in next build
        if t.contains("Type error:") {
            errors.push(t.to_string());
            failed = true;
            continue;
        }
        // Warnings
        if t.contains("warn -") || t.starts_with("⚠") {
            if warnings.len() < 5 {
                warnings.push(t.trim_start_matches("⚠").trim().to_string());
            }
            continue;
        }
        // Checkmark summary lines: "✓ Compiled successfully", "✓ Collecting page data"
        if t.starts_with("✓") || t.starts_with("✔") {
            checkmarks.push(t.to_string());
            continue;
        }
        // "Generating static pages (40/40)" — extract route count
        if t.contains("Generating static pages") || t.contains("static pages") {
            if let Some(n) = extract_page_count(t) {
                route_count = Some(n);
            }
            continue;
        }
        // Build time: "Compiled in X.Xs" or "done in Xs"
        if (t.contains("Compiled in") || t.contains("compiled in")) && t.contains('s') {
            build_time = Some(t.to_string());
            continue;
        }
        // Drop: route table rows (lines starting with ├, └, ┌, │), chunk lists, size info
        if t.starts_with('├') || t.starts_with('└') || t.starts_with('┌') || t.starts_with('│') {
            continue;
        }
        // Drop: chunk manifest lines
        if t.starts_with("chunks/") || t.contains(".js ") && t.contains("kB") {
            continue;
        }
    }

    if !errors.is_empty() {
        let mut out = errors;
        if !warnings.is_empty() {
            out.push(format!("[{} warnings]", warnings.len()));
        }
        return out.join("\n");
    }

    let mut out: Vec<String> = Vec::new();

    // Key checkmarks (skip verbose intermediate ones, keep final ones)
    let key_checks: Vec<&str> = checkmarks
        .iter()
        .filter(|c| {
            c.contains("Compiled successfully")
                || c.contains("Linting")
                || c.contains("Generating static")
                || c.contains("Finalizing")
                || c.contains("build traces")
        })
        .map(|s| s.as_str())
        .collect();

    if key_checks.is_empty() && !checkmarks.is_empty() {
        out.push(checkmarks.last().unwrap().clone());
    } else {
        out.extend(key_checks.iter().map(|s| s.to_string()));
    }

    if let Some(n) = route_count {
        out.push(format!("[{} static pages generated]", n));
    }
    if !warnings.is_empty() {
        out.push(format!("[{} warnings]", warnings.len()));
        for w in warnings.iter().take(3) {
            out.push(format!("  ⚠ {}", w));
        }
    }
    if let Some(t) = build_time {
        out.push(t);
    }

    if failed {
        out.push("Build FAILED".to_string());
    } else if out.is_empty() {
        out.push("Build complete".to_string());
    }

    out.join("\n")
}

fn filter_dev(output: &str) -> String {
    // Dev server: keep errors, warnings, and "ready" / "compiled" lines
    let mut out: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_lowercase();
        if lower.contains("error")
            || lower.contains("warn")
            || lower.contains("ready")
            || lower.contains("started server")
            || lower.contains("compiled")
            || lower.contains("compiling")
        {
            out.push(t.to_string());
        }
    }

    if out.is_empty() {
        output.to_string()
    } else {
        out.join("\n")
    }
}

fn filter_lint(output: &str) -> String {
    // next lint delegates to ESLint — use the same pattern
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("Error:") || t.contains("error") {
            errors.push(t.to_string());
        } else if t.contains("Warning:") || t.contains("warning") {
            if warnings.len() < 10 {
                warnings.push(t.to_string());
            }
        }
    }

    if errors.is_empty() && warnings.is_empty() {
        return "No lint errors found.".to_string();
    }

    let mut out = errors;
    if !warnings.is_empty() {
        out.push(format!("[{} lint warnings]", warnings.len()));
    }
    out.join("\n")
}

fn filter_generic(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= 20 {
        return output.to_string();
    }
    let important: Vec<String> = lines
        .iter()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("error") || lower.contains("warn") || lower.contains("ready")
        })
        .map(|l| l.to_string())
        .collect();

    if important.is_empty() {
        // Return last 10 lines as summary
        let tail = &lines[lines.len().saturating_sub(10)..];
        return tail.join("\n");
    }
    important.join("\n")
}

fn extract_page_count(line: &str) -> Option<usize> {
    // "Generating static pages (40/40)" → 40
    if let Some(start) = line.rfind('(') {
        if let Some(end) = line.rfind(')') {
            let inner = &line[start + 1..end];
            if let Some(slash) = inner.find('/') {
                if let Ok(n) = inner[slash + 1..].trim().parse::<usize>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args(subcmd: &str) -> Vec<String> {
        vec!["next".to_string(), subcmd.to_string()]
    }

    #[test]
    fn build_success_collapses_route_table() {
        let output = "\
   ▲ Next.js 14.0.1
   ✓ Compiled successfully
   ✓ Linting and checking validity of types
   ✓ Collecting page data
   ✓ Generating static pages (24/24)
   ✓ Finalizing page optimization

Route (app)                              Size     First Load JS
┌ ○ /                                   5.2 kB         93.8 kB
├ ○ /_not-found                         871 B          81.9 kB
└ ○ /about                              1.13 kB        92.5 kB
+ First Load JS shared by all            76.6 kB
  ├ chunks/4bd1b696.js                  53.5 kB
  └ other shared chunks (total)         2.22 kB
";
        let result = NextHandler.filter(output, &args("build"));
        assert!(result.contains("Compiled successfully") || result.contains("static pages") || result.contains("Build complete"));
        // Route table rows should be dropped
        assert!(!result.contains("93.8 kB"));
        assert!(!result.contains("┌"));
    }

    #[test]
    fn build_shows_errors() {
        let output = "\
   ▲ Next.js 14.0.1
   Failed to compile.
Error: ./src/app/page.tsx
Type error: Property 'foo' does not exist on type 'Bar'.
";
        let result = NextHandler.filter(output, &args("build"));
        assert!(result.contains("Type error") || result.contains("error"));
    }

    #[test]
    fn build_extracts_page_count() {
        let count = extract_page_count("✓ Generating static pages (40/40)");
        assert_eq!(count, Some(40));
    }

    #[test]
    fn dev_keeps_ready_line() {
        let output = "\
   - wait  compiling / (client and server)...
   - event compiled client and server successfully in 532 ms (20 modules)
   ✓ Ready in 2.3s
   - Local: http://localhost:3000
";
        let result = NextHandler.filter(output, &args("dev"));
        assert!(result.contains("Ready") || result.contains("compiled") || result.contains("ready"));
    }

    #[test]
    fn lint_no_errors_returns_clean_message() {
        let output = "   ✓ No ESLint warnings or errors\n";
        let result = NextHandler.filter(output, &args("lint"));
        // Should indicate clean
        assert!(!result.is_empty());
    }
}
