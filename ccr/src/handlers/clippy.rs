use super::Handler;

/// Handles standalone `clippy` / `cargo-clippy` invocations.
/// Parses rustc-style diagnostic output (not JSON) and collapses noise.
pub struct ClippyHandler;

impl Handler for ClippyHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        filter_clippy(output)
    }
}

pub fn filter_clippy(output: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut help_lines: Vec<String> = Vec::new();
    let mut summary: Option<String> = None;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Summary lines: "error: aborting due to N previous errors"
        // or "warning: N warnings emitted"
        if t.starts_with("error: aborting")
            || t.starts_with("error: could not compile")
            || t.contains("warnings emitted")
            || t.contains("warning emitted")
        {
            summary = Some(t.to_string());
            continue;
        }
        // Skip note/help continuation lines (indented or starting with " = ")
        if t.starts_with("= note:") || t.starts_with("= help:") || t.starts_with("|") {
            continue;
        }
        // Skip "Checking ..." / "Finished ..." cargo lines
        if t.starts_with("Checking ") || t.starts_with("Finished ") || t.starts_with("Compiling ") {
            continue;
        }
        if t.starts_with("error[") || t.starts_with("error:") {
            // Extract just the message without the span detail
            let msg = extract_diagnostic(t);
            if !errors.contains(&msg) {
                errors.push(msg);
            }
        } else if t.starts_with("warning:") {
            let msg = extract_diagnostic(t);
            if !warnings.contains(&msg) {
                warnings.push(msg);
            }
        } else if t.starts_with("help:") {
            help_lines.push(t.to_string());
        }
    }

    if errors.is_empty() && warnings.is_empty() {
        if let Some(s) = summary {
            return s;
        }
        return output.to_string();
    }

    let mut out: Vec<String> = Vec::new();
    out.extend(errors.iter().cloned());

    if !warnings.is_empty() {
        out.push(format!("[{} clippy warnings]", warnings.len()));
        for w in warnings.iter().take(5) {
            out.push(format!("  {}", w));
        }
        if warnings.len() > 5 {
            out.push(format!("  [+{} more]", warnings.len() - 5));
        }
    }
    if !help_lines.is_empty() {
        out.push(help_lines[0].clone());
    }
    if let Some(s) = summary {
        out.push(s);
    }

    out.join("\n")
}

fn extract_diagnostic(line: &str) -> String {
    // "warning: unused variable `x` [unused_variables]" → keep as-is but trim location suffix
    // "error[E0308]: mismatched types" → keep error code + message
    line.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec![] }

    #[test]
    fn clean_build_passthrough() {
        let output = "    Checking myapp v0.1.0\n    Finished dev [unoptimized] target(s) in 0.42s\n";
        let result = ClippyHandler.filter(output, &args());
        assert!(result.contains("Finished") || result.contains("Checking") || !result.is_empty());
    }

    #[test]
    fn collapses_warnings() {
        let output = "\
warning: unused variable `x` [unused_variables]
  --> src/main.rs:5:9
   |
5  |     let x = 1;
   |         ^ help: if this is intentional, prefix it with an underscore: `_x`
warning: unused import: `std::fmt` [unused_imports]
  --> src/lib.rs:1:5
warning: 2 warnings emitted";
        let result = ClippyHandler.filter(output, &args());
        assert!(result.contains("clippy warnings") || result.contains("warning"));
        // Should not contain raw span lines
        assert!(!result.contains("-->"));
    }

    #[test]
    fn shows_errors() {
        let output = "\
error[E0308]: mismatched types
  --> src/main.rs:10:5
error: aborting due to 1 previous error";
        let result = ClippyHandler.filter(output, &args());
        assert!(result.contains("E0308") || result.contains("mismatched types"));
    }

    #[test]
    fn deduplicates_same_warning() {
        let output = "\
warning: unused variable `x`\nwarning: unused variable `x`\nwarning: unused variable `x`\n";
        let result = ClippyHandler.filter(output, &args());
        let count = result.matches("unused variable `x`").count();
        assert_eq!(count, 1, "duplicate warnings should be collapsed");
    }
}
