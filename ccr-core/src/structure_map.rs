//! Structural digest extraction for code files.
//!
//! On unchanged file re-reads, returns only the structural skeleton
//! (function/class/type signatures without bodies) instead of the full content.
//! Lets the model confirm "this file is still the same" without re-reading
//! thousands of lines of implementation.

/// Extract a structural digest from a code file.
///
/// Returns a compact multi-line string with top-level signatures,
/// wrapped with a header: `[structural digest: name.rs - N lines -> M signatures]`
pub fn extract(file_path: &str, content: &str) -> String {
    let line_count = content.lines().count();
    let name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let sigs = match ext {
        "rs" => extract_rust(content),
        "py" => extract_python(content),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => extract_ts_js(content),
        "go" => extract_go(content),
        "java" | "kt" | "scala" => extract_java_like(content),
        "rb" => extract_ruby(content),
        "c" | "cpp" | "h" | "hpp" | "cc" => extract_c_like(content),
        _ => extract_fallback(content),
    };

    let sig_count = sigs.lines().filter(|l| !l.trim().is_empty()).count();
    format!(
        "[structural digest: {} - {} lines -> {} signatures]\n{}",
        name, line_count, sig_count, sigs
    )
}

// -- Language extractors --

fn extract_rust(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = !trimmed.contains("*/");
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Attributes (keep as decorators for context, skip test configs)
        if trimmed.starts_with("#[") && !trimmed.starts_with("#[cfg(test)") {
            out.push(truncate(line, 100));
            continue;
        }

        let is_sig = starts_with_any(
            trimmed,
            &[
                "pub fn ", "fn ", "pub async fn ", "async fn ",
                "pub struct ", "struct ",
                "pub enum ", "enum ",
                "pub trait ", "trait ",
                "pub type ", "type ",
                "impl ", "pub impl ",
                "pub const ", "const ",
                "pub static ", "static ",
                "pub mod ", "mod ",
                "pub use ", "use ",
            ],
        );

        if is_sig {
            // Strip body openings - show just the signature line
            let sig = strip_body(line);
            out.push(truncate(&sig, 120));
        }
    }
    out.join("\n")
}

fn extract_python(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_multiline = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip triple-quoted strings (docstrings)
        if trimmed.contains("\"\"\"") || trimmed.contains("'''") {
            in_multiline = !in_multiline;
            continue;
        }
        if in_multiline {
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let is_sig = trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with('@')          // decorators
            || trimmed.starts_with("import ")
            || trimmed.starts_with("from ");

        if is_sig {
            out.push(truncate(line, 120));
        }
    }
    out.join("\n")
}

fn extract_ts_js(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") || trimmed.starts_with("/**") {
            in_block_comment = !trimmed.contains("*/");
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let is_sig = starts_with_any(
            trimmed,
            &[
                "export default function ",
                "export async function ",
                "export function ",
                "export default class ",
                "export class ",
                "export abstract class ",
                "export const ",
                "export let ",
                "export type ",
                "export interface ",
                "export enum ",
                "function ",
                "async function ",
                "class ",
                "interface ",
                "type ",
                "const ",
                "import ",
                "export {",
                "export * ",
            ],
        );

        if is_sig {
            let sig = strip_body(line);
            out.push(truncate(&sig, 120));
        }
    }
    out.join("\n")
}

fn extract_go(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = !trimmed.contains("*/");
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let is_sig = starts_with_any(
            trimmed,
            &["func ", "type ", "var ", "const ", "package ", "import"],
        );
        if is_sig {
            let sig = strip_body(line);
            out.push(truncate(&sig, 120));
        }
    }
    out.join("\n")
}

fn extract_java_like(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") || trimmed.starts_with("/**") {
            in_block_comment = !trimmed.contains("*/");
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Keep: class/interface/enum declarations, method signatures, annotations
        let is_sig = trimmed.starts_with('@')
            || trimmed.contains("class ")
            || trimmed.contains("interface ")
            || trimmed.contains("enum ")
            || (trimmed.contains('(') && trimmed.contains(')') && !trimmed.starts_with("//"));

        if is_sig {
            let sig = strip_body(line);
            out.push(truncate(&sig, 120));
        }
    }
    out.join("\n")
}

fn extract_ruby(content: &str) -> String {
    let mut out = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let is_sig = starts_with_any(
            trimmed,
            &["def ", "class ", "module ", "attr_", "require", "include "],
        );
        if is_sig {
            out.push(truncate(line, 120));
        }
    }
    out.join("\n")
}

fn extract_c_like(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = !trimmed.contains("*/");
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Preprocessor directives, struct/class/function signatures
        let is_sig = trimmed.starts_with('#')
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("typedef ")
            || trimmed.starts_with("namespace ")
            || (trimmed.contains('(')
                && !trimmed.starts_with("if ")
                && !trimmed.starts_with("while ")
                && !trimmed.starts_with("for "));

        if is_sig {
            let sig = strip_body(line);
            out.push(truncate(&sig, 120));
        }
    }
    out.join("\n")
}

/// For unknown file types: return first 20 lines + last 10 lines with a gap marker.
fn extract_fallback(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 40 {
        return content.to_string();
    }
    let head: Vec<&str> = lines.iter().take(20).copied().collect();
    let tail: Vec<&str> = lines.iter().rev().take(10).rev().copied().collect();
    format!(
        "{}\n[... {} lines omitted ...]\n{}",
        head.join("\n"),
        lines.len() - 30,
        tail.join("\n")
    )
}

// -- Helpers --

fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

/// Strip the opening brace / body from a signature line.
/// `fn foo() {` -> `fn foo()`
/// `class Bar extends Base {` -> `class Bar extends Base`
fn strip_body(line: &str) -> String {
    // Remove trailing ` {` or ` {` variants but keep `;` signatures
    let s = line.trim_end();
    if s.ends_with('{') {
        s.trim_end_matches('{').trim_end().to_string()
    } else if s.ends_with("{}") {
        s.trim_end_matches("{}").trim_end().to_string() + " {}"
    } else {
        s.to_string()
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..max_chars.saturating_sub(3)])
    }
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_extracts_fn_signatures() {
        let code = r#"
use std::io;

pub struct Foo {
    bar: u32,
}

impl Foo {
    pub fn new(bar: u32) -> Self {
        Self { bar }
    }
    fn private_helper(&self) -> u32 {
        self.bar
    }
}

pub enum Status { Ok, Err }

pub trait MyTrait {
    fn required(&self);
}
"#;
        let result = extract("test.rs", code);
        assert!(result.contains("pub struct Foo"), "missing struct: {result}");
        assert!(result.contains("impl Foo"), "missing impl: {result}");
        assert!(result.contains("pub fn new"), "missing pub fn: {result}");
        assert!(result.contains("fn private_helper"), "missing private fn: {result}");
        assert!(result.contains("pub enum Status"), "missing enum: {result}");
        assert!(result.contains("pub trait MyTrait"), "missing trait: {result}");
    }

    #[test]
    fn python_extracts_def_class() {
        let code = r#"
import os
from pathlib import Path

class MyClass:
    def __init__(self, x):
        self.x = x

    def method(self):
        return self.x

async def async_fn():
    pass
"#;
        let result = extract("module.py", code);
        assert!(result.contains("class MyClass"), "missing class: {result}");
        assert!(result.contains("def __init__"), "missing __init__: {result}");
        assert!(result.contains("def method"), "missing method: {result}");
        assert!(result.contains("async def async_fn"), "missing async def: {result}");
        assert!(result.contains("import os"), "missing import: {result}");
    }

    #[test]
    fn ts_extracts_exports() {
        let code = r#"
import { foo } from './foo';
export interface Config { name: string; }
export type Handler = (req: Request) => void;
export class Server {
  constructor() {}
}
export function start(cfg: Config): void {
  console.log(cfg.name);
}
export const DEFAULT_PORT = 3000;
"#;
        let result = extract("server.ts", code);
        assert!(result.contains("export interface Config"), "missing interface: {result}");
        assert!(result.contains("export type Handler"), "missing type: {result}");
        assert!(result.contains("export class Server"), "missing class: {result}");
        assert!(result.contains("export function start"), "missing function: {result}");
        assert!(result.contains("export const DEFAULT_PORT"), "missing const: {result}");
    }

    #[test]
    fn header_shows_file_name_and_counts() {
        let code = "fn a() {}\nfn b() {}\n";
        let result = extract("foo.rs", code);
        assert!(result.starts_with("[structural digest: foo.rs"), "bad header: {result}");
        assert!(result.contains("lines ->"), "missing line count: {result}");
    }

    #[test]
    fn empty_file_no_panic() {
        let result = extract("empty.rs", "");
        assert!(result.contains("structural digest"), "should still have header: {result}");
    }

    #[test]
    fn fallback_uses_head_tail_for_unknown_ext() {
        let content: String = (1..=60).map(|i| format!("line {}\n", i)).collect();
        let result = extract("data.xyz", &content);
        assert!(result.contains("line 1"), "should have first line: {result}");
        assert!(result.contains("line 60"), "should have last line: {result}");
        assert!(result.contains("omitted"), "should have gap marker: {result}");
    }
}
