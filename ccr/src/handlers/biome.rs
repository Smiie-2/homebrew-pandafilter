use super::Handler;

/// Handler for Biome — fast JS/TS linter and formatter (successor to Rome).
/// Biome diagnostics include verbose code context snippets (│ lines, ^^^ underlines).
/// This handler strips those snippets, keeping only file:line, rule name, and message.
pub struct BiomeHandler;

impl Handler for BiomeHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        filter_biome(output)
    }
}

pub fn filter_biome(output: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_diagnostic = false;
    let mut kept_header = false;

    for line in output.lines() {
        let t = line.trim();

        // Blank lines are always skipped; in_diagnostic persists through them
        // because Biome's code context appears after blank lines within the block.
        if t.is_empty() {
            continue;
        }

        // Summary line: "Found N diagnostics" / "Checked N files in Xms" / "Fixed N files"
        if t.starts_with("Found ")
            || t.starts_with("Checked ")
            || t.starts_with("Fixed ")
            || t.starts_with("Formatted ")
            || (t.contains("diagnostics") && !t.contains("│"))
            || t.starts_with("The number of diagnostics")
        {
            out.push(t.to_string());
            continue;
        }

        // Diagnostic header line: "./path/to/file.tsx:line:col rulecategory/ruleName ━━━━"
        // Biome uses ━ (heavy horizontal) to underline the header
        if (t.contains(".tsx:") || t.contains(".ts:") || t.contains(".js:") || t.contains(".jsx:") || t.contains(".css:"))
            && t.contains(':')
        {
            // Strip the ━━━ separator from the end
            let clean = t.trim_end_matches('━').trim();
            out.push(clean.to_string());
            in_diagnostic = true;
            kept_header = false;
            continue;
        }

        if !in_diagnostic {
            // Outside diagnostics: keep short non-noise lines
            if !t.starts_with("│") && !t.starts_with("┌") && !t.starts_with("└") && !t.starts_with("×") {
                out.push(t.to_string());
            }
            continue;
        }

        // Inside a diagnostic block:

        // Keep the ✖/✔/ℹ message line (the human-readable description)
        if (t.starts_with("✖") || t.starts_with("✔") || t.starts_with("ℹ") || t.starts_with("×"))
            && !kept_header
        {
            // Strip unicode prefix and keep the message
            let msg = t.trim_start_matches(|c: char| !c.is_alphabetic()).trim();
            if !msg.is_empty() {
                out.push(format!("  {}", msg));
                kept_header = true;
            }
            continue;
        }

        // Drop code context lines: "  N │ code...", "    │ ^^^..."
        if t.contains('│') || t.starts_with('^') {
            continue;
        }

        // Drop fix suggestion lines (ℹ Unsafe fix / Safe fix)
        if t.starts_with("ℹ") || t.starts_with("i ") {
            continue;
        }
    }

    if out.is_empty() {
        return output.to_string();
    }

    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args() -> Vec<String> { vec!["biome".to_string(), "lint".to_string()] }

    #[test]
    fn strips_code_context_keeps_rule_and_message() {
        let output = "\
./src/App.tsx:15:5 lint/correctness/noUnusedVariables ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  ✖ This variable is unused.

  14 │ function App() {
  15 │   const unusedVar = \"hello\";
     │   ^^^^^^^^^^^^^^^^^^^^^^^^^^^
  16 │   return <div>Hello</div>;

  ℹ Unsafe fix: prefix with an underscore.

Checked 42 files in 234ms.
Found 1 diagnostics.
";
        let result = BiomeHandler.filter(output, &args());
        assert!(result.contains("noUnusedVariables"));
        assert!(result.contains("unused"));
        assert!(!result.contains("│")); // code context stripped
        assert!(result.contains("Checked 42 files"));
        assert!(result.lines().count() < output.lines().count() / 2);
    }

    #[test]
    fn keeps_summary_lines() {
        let output = "Checked 150 files in 512ms.\nFound 0 diagnostics.\n";
        let result = BiomeHandler.filter(output, &args());
        assert!(result.contains("Checked 150"));
        assert!(result.contains("0 diagnostics"));
    }

    #[test]
    fn handles_multiple_diagnostics() {
        let output = "\
./src/a.tsx:10:1 lint/a11y/useAltText ━━━━━━━━━

  ✖ Provide alternative text.

   9 │ return (
  10 │   <img src=\"logo.png\" />
     │   ^^^^^^^^^^^^^^^^^^^^^^

./src/b.tsx:5:3 lint/correctness/noUnusedVariables ━━━━━━━━━

  ✖ This variable is unused.

   4 │ function Foo() {
   5 │   const x = 1;
     │   ^^^^^^^^^^^^

Found 2 diagnostics.
";
        let result = BiomeHandler.filter(output, &args());
        assert!(result.contains("useAltText"));
        assert!(result.contains("noUnusedVariables"));
        assert!(!result.contains("│"));
        assert!(result.lines().count() < output.lines().count() / 2);
    }
}
