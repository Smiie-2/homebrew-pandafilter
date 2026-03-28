use super::Handler;

/// Handler for Turborepo — high-performance monorepo build system.
/// Strips verbose per-package inner output; keeps cache status, errors, and final summary.
pub struct TurboHandler;

impl Handler for TurboHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        let mut scope_lines: Vec<String> = Vec::new();
        let mut task_statuses: Vec<String> = Vec::new(); // "pkg:task: cache hit/miss"
        let mut errors: Vec<String> = Vec::new();
        let mut summary_lines: Vec<String> = Vec::new(); // Tasks/Cached/Time block
        let mut in_summary = false;
        let mut in_error = false;
        let mut error_buf: Vec<String> = Vec::new();
        let mut current_task: Option<String> = None;

        for line in output.lines() {
            let t = line.trim();

            // Scope / header lines
            if t.starts_with("•") || t.starts_with("●") {
                scope_lines.push(t.to_string());
                continue;
            }

            // Final summary block (Tasks/Cached/Time)
            if t.starts_with("Tasks:") || t.starts_with("Cached:") || t.starts_with("Time:") || in_summary {
                in_summary = true;
                summary_lines.push(t.to_string());
                if t.starts_with("Time:") {
                    in_summary = false;
                }
                continue;
            }

            // Per-task lines: "pkg:task: ..."
            let task_prefix = extract_task_prefix(t);
            if let Some(ref prefix) = task_prefix {
                let rest = t[prefix.len()..].trim();

                // Cache hit/miss — always keep, one line per task
                if rest.starts_with("cache hit") || rest.starts_with("cache miss") {
                    let short = rest.split(',').next().unwrap_or(rest);
                    task_statuses.push(format!("{} {}", prefix.trim_end_matches(':'), short));
                    current_task = Some(prefix.clone());
                    in_error = false;
                    continue;
                }

                // Error line inside a task
                if rest.to_lowercase().contains("error") && !rest.starts_with('>') {
                    in_error = true;
                    error_buf.push(format!("{} {}", prefix.trim_end_matches(':'), rest));
                    continue;
                }

                if in_error {
                    if rest.is_empty() {
                        in_error = false;
                        errors.extend(error_buf.drain(..));
                    } else if error_buf.len() < 8 {
                        error_buf.push(format!("  {}", rest));
                    }
                    continue;
                }

                // Drop all other inner task output (build steps, npm scripts, etc.)
                let _ = current_task.as_ref();
                continue;
            }

            // Non-task lines outside summary: keep if short/meaningful
            if t.is_empty() {
                continue;
            }
        }

        if !error_buf.is_empty() {
            errors.extend(error_buf);
        }

        if task_statuses.is_empty() && errors.is_empty() && summary_lines.is_empty() {
            return output.to_string();
        }

        let mut out: Vec<String> = Vec::new();
        out.extend(scope_lines);
        out.extend(task_statuses);
        if !errors.is_empty() {
            out.push(String::new());
            out.extend(errors);
        }
        if !summary_lines.is_empty() {
            out.push(String::new());
            out.extend(summary_lines);
        }
        out.join("\n")
    }
}

/// Extract "pkg:task: " prefix from a turbo output line, or None if it's not a task line.
fn extract_task_prefix(line: &str) -> Option<String> {
    // Pattern: "name:subcommand: " where name can contain @scope/pkg
    // e.g. "@repo/ui:build: " or "web:dev: "
    let mut colon_count = 0;
    let mut end = 0;
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' {
            colon_count += 1;
            if colon_count == 2 {
                // Check that next char is space
                if bytes.get(i + 1) == Some(&b' ') {
                    end = i + 2;
                    break;
                }
            }
        } else if b == b' ' && colon_count < 2 {
            // Space before second colon — not a task line
            return None;
        }
    }
    if end > 0 && end < line.len() {
        Some(line[..end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec!["turbo".to_string(), "run".to_string(), "build".to_string()] }

    #[test]
    fn keeps_cache_statuses_and_summary() {
        let output = "\
• Packages in scope: web, docs, @repo/ui
• Running build in 3 packages
• Remote caching disabled

@repo/ui:build: cache miss, executing 1234abcd
@repo/ui:build: > @repo/ui@0.0.0 build
@repo/ui:build: > tsc
@repo/ui:build:
web:build: cache hit, replaying output 9876fedc
web:build: > next build
web:build:  ✓ Compiled successfully
docs:build: cache miss, executing abcd5678

 Tasks:    3 successful, 3 total
  Cached:    1 cached, 3 total
    Time:    15.234s >>> FULL TURBO
";
        let result = TurboHandler.filter(output, &args());
        assert!(result.contains("cache hit"));
        assert!(result.contains("cache miss"));
        assert!(result.contains("Tasks:"));
        assert!(result.contains("Time:"));
        // Inner task output should be dropped
        assert!(!result.contains("tsc"));
        assert!(!result.contains("next build"));
        assert!(result.lines().count() < output.lines().count());
    }

    #[test]
    fn extract_task_prefix_works() {
        assert_eq!(
            extract_task_prefix("@repo/ui:build: cache miss"),
            Some("@repo/ui:build: ".to_string())
        );
        assert_eq!(
            extract_task_prefix("web:dev: > npm start"),
            Some("web:dev: ".to_string())
        );
        assert_eq!(extract_task_prefix("• Packages in scope"), None);
        assert_eq!(extract_task_prefix(" Tasks:    3 successful"), None);
    }
}
