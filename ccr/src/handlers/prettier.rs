use super::Handler;

/// Handler for Prettier — opinionated code formatter.
/// Compresses `--check` output (which lists every file) and `--write` output.
pub struct PrettierHandler;

impl Handler for PrettierHandler {
    fn filter(&self, output: &str, args: &[String]) -> String {
        let is_check = args.iter().any(|a| a == "--check" || a == "-c");
        let is_write = args.iter().any(|a| a == "--write" || a == "-w");

        if is_check {
            filter_check(output)
        } else if is_write {
            filter_write(output)
        } else {
            // Infer from output content
            if output.contains("All matched files use Prettier code style!")
                || output.contains("[warn]")
                || output.contains("Checking formatting")
            {
                filter_check(output)
            } else {
                filter_write(output)
            }
        }
    }
}

fn filter_check(output: &str) -> String {
    let mut needs_formatting: Vec<String> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();
    let mut all_clean = false;

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() || t == "Checking formatting..." {
            continue;
        }
        // All clean
        if t.contains("All matched files use Prettier code style!") {
            all_clean = true;
            continue;
        }
        // Files needing formatting: "[warn] path/to/file.js"
        if t.starts_with("[warn]") {
            let file = t.trim_start_matches("[warn]").trim();
            // Skip the summary "[warn] Found N files which need formatting."
            if file.starts_with("Found ") {
                continue;
            }
            needs_formatting.push(file.to_string());
            continue;
        }
        // Parse errors
        if t.contains("SyntaxError") || t.contains("error") {
            parse_errors.push(t.to_string());
            continue;
        }
        // "Code style issues found in N files. Forgot to run Prettier?"
        if t.contains("Code style issues") {
            // Extract count for summary
            continue;
        }
    }

    if !parse_errors.is_empty() {
        return parse_errors.join("\n");
    }

    if all_clean && needs_formatting.is_empty() {
        return "All files formatted correctly.".to_string();
    }

    if needs_formatting.is_empty() {
        return output.to_string();
    }

    let mut out: Vec<String> = Vec::new();
    out.push(format!("[{} file(s) need formatting]", needs_formatting.len()));
    for f in needs_formatting.iter().take(10) {
        out.push(format!("  {}", f));
    }
    if needs_formatting.len() > 10 {
        out.push(format!("  [+{} more]", needs_formatting.len() - 10));
    }
    out.push("Run `prettier --write .` to fix.".to_string());
    out.join("\n")
}

fn filter_write(output: &str) -> String {
    let mut written: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("SyntaxError") || t.contains("error") {
            errors.push(t.to_string());
            continue;
        }
        // Written files: "src/index.js 45ms"  or just "src/index.js"
        // Prettier --write prints each file it processed
        if !t.starts_with('[') && !t.starts_with('(') && (t.contains('/') || t.contains('.')) {
            written.push(t.to_string());
        }
    }

    if !errors.is_empty() {
        return errors.join("\n");
    }

    if written.is_empty() {
        return output.to_string();
    }

    format!("[{} file(s) formatted]", written.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn check_args() -> Vec<String> {
        vec!["prettier".to_string(), "--check".to_string(), ".".to_string()]
    }

    fn write_args() -> Vec<String> {
        vec!["prettier".to_string(), "--write".to_string(), ".".to_string()]
    }

    #[test]
    fn check_all_clean() {
        let output = "\
Checking formatting...
All matched files use Prettier code style!
";
        let result = PrettierHandler.filter(output, &check_args());
        assert!(result.contains("formatted correctly") || result.contains("Prettier code style"));
    }

    #[test]
    fn check_files_need_formatting() {
        let output = "\
Checking formatting...
[warn] src/index.js
[warn] src/components/App.js
[warn] src/utils/helpers.js
[warn] Found 3 files which need formatting.
";
        let result = PrettierHandler.filter(output, &check_args());
        assert!(result.contains("3 file(s) need formatting"));
        assert!(result.contains("src/index.js"));
        assert!(!result.contains("Found 3 files")); // summary line dropped
    }

    #[test]
    fn check_many_files_truncates() {
        let files: Vec<String> = (0..15)
            .map(|i| format!("[warn] src/file{}.js", i))
            .collect();
        let output = format!("Checking formatting...\n{}\n", files.join("\n"));
        let result = PrettierHandler.filter(&output, &check_args());
        assert!(result.contains("15 file(s) need formatting"));
        assert!(result.contains("[+5 more]"));
    }

    #[test]
    fn write_reports_count() {
        let output = "\
src/index.js 45ms
src/components/App.js 23ms
src/utils/helpers.js 12ms
";
        let result = PrettierHandler.filter(output, &write_args());
        assert!(result.contains("3 file(s) formatted"));
    }

    #[test]
    fn check_inferred_without_flag() {
        let output = "\
Checking formatting...\n[warn] src/foo.js\n[warn] Found 1 files which need formatting.\n";
        // No --check flag, but content implies check mode
        let no_flag_args = vec!["prettier".to_string(), ".".to_string()];
        let result = PrettierHandler.filter(output, &no_flag_args);
        assert!(result.contains("need formatting") || result.contains("foo.js"));
    }
}
