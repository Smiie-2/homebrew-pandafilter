use super::Handler;

/// Handler for Stylelint — CSS/SCSS/Less linter.
/// Groups issues by file; caps at 40 total; shows summary count.
pub struct StylelintHandler;

const MAX_ISSUES: usize = 40;

impl Handler for StylelintHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        if output.trim().is_empty() {
            return "No issues found.".to_string();
        }

        let mut current_file: Option<String> = None;
        let mut file_issues: Vec<String> = Vec::new();
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        let mut total = 0usize;
        let mut summary: Option<String> = None;

        for line in output.lines() {
            let t = line.trim();

            if t.is_empty() {
                // Flush current file
                if let Some(f) = current_file.take() {
                    if !file_issues.is_empty() {
                        grouped.push((f, std::mem::take(&mut file_issues)));
                    }
                }
                continue;
            }

            // Summary line: "N problems (M errors, P warnings)"
            if t.chars().next().map_or(false, |c| c.is_ascii_digit())
                && t.contains("problem")
            {
                summary = Some(t.to_string());
                continue;
            }

            // File path line: not indented, looks like a path
            if !line.starts_with(' ') && !line.starts_with('\t')
                && (t.contains('/') || t.contains('\\') || t.ends_with(".css") || t.ends_with(".scss") || t.ends_with(".less"))
            {
                // Flush previous file
                if let Some(f) = current_file.take() {
                    if !file_issues.is_empty() {
                        grouped.push((f, std::mem::take(&mut file_issues)));
                    }
                }
                current_file = Some(t.to_string());
                continue;
            }

            // Issue line: indented with line:col info
            if line.starts_with(' ') || line.starts_with('\t') {
                let lower = t.to_lowercase();
                if lower.contains("error") || lower.contains("warning") || lower.contains("✖") || lower.contains("⚠") {
                    total += 1;
                    if total <= MAX_ISSUES {
                        file_issues.push(t.to_string());
                    }
                }
            }
        }

        // Flush last file
        if let Some(f) = current_file {
            if !file_issues.is_empty() {
                grouped.push((f, file_issues));
            }
        }

        if grouped.is_empty() {
            if let Some(s) = summary {
                return s;
            }
            // All clean
            return if output.contains("0 problems") || output.trim().is_empty() {
                "No issues found.".to_string()
            } else {
                output.to_string()
            };
        }

        let mut out: Vec<String> = Vec::new();
        for (file, issues) in &grouped {
            out.push(file.clone());
            for issue in issues {
                out.push(format!("  {}", issue));
            }
        }

        if total > MAX_ISSUES {
            out.push(format!("[+{} more issues]", total - MAX_ISSUES));
        }

        if let Some(s) = summary {
            out.push(s);
        } else {
            out.push(format!("[{} issue(s) found]", total));
        }

        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec!["stylelint".to_string()] }

    #[test]
    fn groups_by_file_and_shows_summary() {
        let output = "\
src/components/Button.css
  3:5  ✖  Unexpected unknown property \"colour\"  property-no-unknown
  7:3  ⚠  Expected a leading zero  number-leading-zero

src/styles/global.scss
 12:1  ✖  Unexpected empty block  block-no-empty

3 problems (2 errors, 1 warning)
";
        let result = StylelintHandler.filter(output, &args());
        assert!(result.contains("Button.css"));
        assert!(result.contains("global.scss"));
        assert!(result.contains("3 problems") || result.contains("3 issue"));
        assert!(!result.is_empty());
    }

    #[test]
    fn empty_output_is_clean() {
        let result = StylelintHandler.filter("", &args());
        assert!(result.contains("No issues") || result.is_empty() || result.contains("0"));
    }

    #[test]
    fn caps_at_max_issues() {
        let mut output = String::new();
        for i in 0..20 {
            output.push_str(&format!("src/file{}.css\n", i));
            for j in 0..5 {
                output.push_str(&format!("  {}:1  ✖  Some error rule-name\n", j + 1));
            }
            output.push('\n');
        }
        output.push_str("100 problems (100 errors, 0 warnings)\n");
        let result = StylelintHandler.filter(&output, &args());
        let issue_lines = result.lines().filter(|l| l.trim().starts_with('✖')).count();
        assert!(issue_lines <= MAX_ISSUES);
        assert!(result.contains("more issues"));
    }
}
