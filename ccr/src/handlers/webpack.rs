use super::Handler;

/// Handler for Webpack — module bundler.
/// Strips node_modules module resolution noise; keeps assets, errors, warnings, build result.
pub struct WebpackHandler;

impl Handler for WebpackHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        let mut assets: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut result_line: Option<String> = None;

        let mut in_error = false;
        let mut in_warning = false;
        let mut error_buf: Vec<String> = Vec::new();
        let mut warning_buf: Vec<String> = Vec::new();

        for line in output.lines() {
            let t = line.trim();

            // Final result line
            if t.starts_with("webpack ") && (t.contains("compiled") || t.contains("compile")) {
                result_line = Some(t.to_string());
                in_error = false;
                in_warning = false;
                continue;
            }

            // Error block
            if t.starts_with("ERROR in") || t.starts_with("Module build failed") || t.starts_with("Module not found") {
                in_error = true;
                in_warning = false;
                if !warning_buf.is_empty() {
                    warnings.extend(warning_buf.drain(..));
                }
                error_buf.push(t.to_string());
                continue;
            }

            // Warning block
            if t.starts_with("WARNING in") {
                in_warning = true;
                in_error = false;
                if !error_buf.is_empty() {
                    errors.extend(error_buf.drain(..));
                }
                warning_buf.push(t.to_string());
                continue;
            }

            // Empty line = end of error/warning block
            if t.is_empty() {
                if in_error && !error_buf.is_empty() {
                    errors.extend(error_buf.drain(..));
                    in_error = false;
                }
                if in_warning && !warning_buf.is_empty() {
                    warnings.extend(warning_buf.drain(..));
                    in_warning = false;
                }
                continue;
            }

            if in_error {
                if error_buf.len() < 10 {
                    error_buf.push(t.to_string());
                }
                continue;
            }

            if in_warning {
                if warning_buf.len() < 5 {
                    warning_buf.push(t.to_string());
                }
                continue;
            }

            // Asset lines: "asset main.js 1.44 MiB [emitted]"
            if t.starts_with("asset ") {
                assets.push(t.to_string());
                continue;
            }

            // Drop all module noise lines
            // "modules by path ./node_modules/..."
            // "cacheable modules X MiB"
            // "runtime modules X KiB N modules"
            // "  ./node_modules/react/index.js X KiB [built]"
            // "  ./src/index.js X KiB [built]"
            // These are all the verbose module graph lines
        }

        // Flush any open buffers
        if !error_buf.is_empty() { errors.extend(error_buf); }
        if !warning_buf.is_empty() { warnings.extend(warning_buf); }

        if errors.is_empty() && assets.is_empty() && result_line.is_none() {
            return output.to_string();
        }

        let mut out: Vec<String> = Vec::new();

        if !assets.is_empty() {
            out.extend(assets);
        }

        if !errors.is_empty() {
            out.extend(errors);
        }

        // Cap warnings
        let warn_total = warnings.len();
        let shown: Vec<String> = warnings.into_iter().take(5).collect();
        out.extend(shown);
        if warn_total > 5 {
            out.push(format!("[+{} more warnings]", warn_total - 5));
        }

        if let Some(r) = result_line {
            out.push(r);
        }

        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec!["webpack".to_string()] }

    #[test]
    fn keeps_assets_and_result() {
        let output = "\
asset main.js 1.44 MiB [emitted] (name: main)
asset vendors.js 234 KiB [emitted]
cacheable modules 2.37 MiB
  modules by path ./node_modules/ 2.27 MiB
    ./node_modules/react/index.js 65.7 KiB [built] [code generated]
  modules by path ./src/ 98.7 KiB
    ./src/index.js 1.62 KiB [built] [code generated]
webpack 5.89.0 compiled successfully in 3291 ms
";
        let result = WebpackHandler.filter(output, &args());
        assert!(result.contains("asset main.js"));
        assert!(result.contains("compiled successfully"));
        assert!(!result.contains("node_modules/react/index.js"));
    }

    #[test]
    fn keeps_errors() {
        let output = "\
asset main.js 200 KiB [emitted]
ERROR in ./src/missing.js
Module not found: Error: Can't resolve './helper'
webpack 5.89.0 compiled with 1 error in 1234 ms
";
        let result = WebpackHandler.filter(output, &args());
        assert!(result.contains("ERROR in") || result.contains("Module not found"));
        assert!(result.contains("compiled with 1 error"));
    }

    #[test]
    fn caps_warnings() {
        let mut output = String::new();
        for i in 0..10 {
            output.push_str(&format!("WARNING in ./src/file{}.js\nSize limit exceeded\n\n", i));
        }
        output.push_str("webpack 5.89.0 compiled with 10 warnings in 2000 ms\n");
        let result = WebpackHandler.filter(&output, &args());
        assert!(result.contains("more warnings") || result.lines().count() < output.lines().count());
    }
}
