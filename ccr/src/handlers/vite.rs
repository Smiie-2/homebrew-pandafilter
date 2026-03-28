use super::Handler;

/// Handler for Vite — fast build tool for modern frontend projects.
/// Covers `vite build` (asset table + build time) and `vite dev` (HMR noise).
pub struct ViteHandler;

impl Handler for ViteHandler {
    fn filter(&self, output: &str, args: &[String]) -> String {
        let is_build = args.iter().any(|a| a == "build")
            || output.contains("building for production")
            || output.contains("building for staging");

        if is_build {
            filter_build(output)
        } else {
            filter_dev(output)
        }
    }
}

fn filter_build(output: &str) -> String {
    let mut header: Option<String> = None;
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut assets: Vec<String> = Vec::new();
    let mut summary: Option<String> = None; // "✓ built in Xs"
    let mut modules_line: Option<String> = None; // "✓ N modules transformed."

    let mut in_error = false;
    let mut error_buf: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();

        // Build header: "vite v5.0.0 building for production..."
        if t.starts_with("vite v") {
            header = Some(t.to_string());
            continue;
        }

        // Modules transformed: keep as one-liner
        if t.contains("modules transformed") {
            modules_line = Some(t.trim_start_matches('✓').trim().to_string());
            continue;
        }

        // Progress indicators (drop)
        if t.starts_with("rendering chunks")
            || t.starts_with("transforming")
            || t.starts_with("computing module graph")
            || t.starts_with("analyzing")
        {
            continue;
        }

        // Error block start
        if t.starts_with("✗") || (t.starts_with("error") && !t.starts_with("error:")) {
            in_error = true;
            error_buf.push(line.to_string());
            continue;
        }
        if in_error {
            if t.is_empty() {
                in_error = false;
                errors.extend(error_buf.drain(..));
            } else {
                // Keep first 8 lines of error, drop stack trace
                if error_buf.len() < 8 {
                    error_buf.push(line.to_string());
                }
            }
            continue;
        }

        // Warnings
        if t.starts_with("warn") || t.starts_with("[WARN]") || t.starts_with("(!)") {
            if warnings.len() < 5 {
                warnings.push(t.to_string());
            }
            continue;
        }

        // Asset output lines: contain " kB " or " MiB " with a path
        if (t.contains(" kB ") || t.contains(" MiB ") || t.contains(" B "))
            && (t.contains('/') || t.contains('\\') || t.starts_with("dist"))
        {
            assets.push(t.to_string());
            continue;
        }

        // Build summary: "✓ built in 3.42s"
        if t.starts_with("✓ built in") || t.starts_with("built in") {
            summary = Some(t.trim_start_matches('✓').trim().to_string());
            continue;
        }
    }

    if !error_buf.is_empty() {
        errors.extend(error_buf);
    }

    // Pass through if nothing actionable extracted
    if errors.is_empty() && assets.is_empty() && summary.is_none() {
        return output.to_string();
    }

    let mut out: Vec<String> = Vec::new();

    if let Some(h) = header {
        out.push(h);
    }
    if let Some(m) = modules_line {
        out.push(m);
    }

    if !errors.is_empty() {
        out.extend(errors);
    }

    if !warnings.is_empty() {
        out.extend(warnings);
    }

    // Collapse asset table: if ≤15 assets show all, else show first 5 + count + last 2
    if assets.len() <= 15 {
        out.extend(assets);
    } else {
        let total = assets.len();
        out.extend(assets[..5].iter().cloned());
        out.push(format!("  ... [{} more chunks] ...", total - 7));
        out.extend(assets[total - 2..].iter().cloned());
    }

    if let Some(s) = summary {
        out.push(s);
    }

    out.join("\n")
}

fn filter_dev(output: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut hmr_count = 0usize;
    let mut last_hmr: Option<String> = None;
    let mut errors: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }

        // HMR noise — deduplicate
        if t.starts_with("[vite] hmr update") || t.starts_with("[vite] page reload") {
            hmr_count += 1;
            last_hmr = Some(t.to_string());
            continue;
        }

        // Errors
        if t.contains("error") || t.starts_with("✗") {
            errors.push(t.to_string());
            continue;
        }

        // Ready / local URL lines — keep
        if t.starts_with("VITE")
            || t.starts_with("➜")
            || t.starts_with("Local:")
            || t.starts_with("Network:")
        {
            out.push(t.to_string());
            continue;
        }

        // Everything else: keep
        out.push(t.to_string());
    }

    if hmr_count > 0 {
        if let Some(last) = last_hmr {
            out.push(if hmr_count == 1 {
                last
            } else {
                format!("[vite] hmr ×{}", hmr_count)
            });
        }
    }

    out.extend(errors);

    if out.is_empty() {
        output.to_string()
    } else {
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn build_args() -> Vec<String> {
        vec!["vite".to_string(), "build".to_string()]
    }
    fn dev_args() -> Vec<String> {
        vec!["vite".to_string(), "dev".to_string()]
    }

    #[test]
    fn build_keeps_assets_and_summary() {
        let output = "\
vite v5.0.8 building for production...
✓ 1847 modules transformed.
dist/index.html         0.46 kB │ gzip:  0.30 kB
dist/assets/index.js  298.36 kB │ gzip: 87.34 kB
✓ built in 3.42s
";
        let result = ViteHandler.filter(output, &build_args());
        assert!(result.contains("built in 3.42s"));
        assert!(result.contains("dist/assets/index.js"));
        assert!(result.contains("1847 modules"));
    }

    #[test]
    fn build_collapses_large_asset_table() {
        let mut output = "vite v5.0.8 building for production...\n✓ 2000 modules transformed.\n".to_string();
        for i in 0..30 {
            output.push_str(&format!("dist/assets/chunk-{}.js  10.00 kB │ gzip: 3.50 kB\n", i));
        }
        output.push_str("✓ built in 8.12s\n");
        let result = ViteHandler.filter(&output, &build_args());
        assert!(result.contains("more chunks"));
        assert!(result.contains("built in 8.12s"));
        // Should be much shorter than input
        assert!(result.lines().count() < output.lines().count() / 2);
    }

    #[test]
    fn dev_deduplicates_hmr() {
        let output = "\
VITE v5.0.8  ready in 312 ms
[vite] hmr update /src/App.tsx
[vite] hmr update /src/App.tsx
[vite] hmr update /src/App.tsx
[vite] page reload /src/main.tsx
[vite] hmr update /src/Button.tsx
";
        let result = ViteHandler.filter(output, &dev_args());
        assert!(result.contains("VITE"));
        // HMR lines should be collapsed
        let hmr_lines = result.lines().filter(|l| l.contains("hmr") || l.contains("reload")).count();
        assert!(hmr_lines <= 2);
    }

    #[test]
    fn build_keeps_errors() {
        let output = "\
vite v5.0.8 building for production...
✗ Build failed in 1.23s
error during build:
src/App.tsx:15:5: ERROR: Transform failed
";
        let result = ViteHandler.filter(output, &build_args());
        assert!(result.contains("Build failed") || result.contains("error"));
    }
}
