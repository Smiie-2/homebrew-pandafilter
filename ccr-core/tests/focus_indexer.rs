//! Unit tests for focus indexer — parsing, cochange building, skipping logic.

use panda_core::focus::indexer::{
    should_skip_dir, should_skip_file, parse_cochange_log, build_cochange_pairs,
};
use std::path::Path;

#[test]
fn test_should_skip_dir_vendor_dirs() {
    assert!(should_skip_dir("node_modules"));
    assert!(should_skip_dir("target"));
    assert!(should_skip_dir(".git"));
    assert!(should_skip_dir("vendor"));
    assert!(should_skip_dir("__pycache__"));
}

#[test]
fn test_should_skip_dir_allowed_dirs() {
    assert!(!should_skip_dir("src"));
    assert!(!should_skip_dir("lib"));
    assert!(!should_skip_dir("tests"));
    assert!(!should_skip_dir("benches"));
    assert!(!should_skip_dir("examples"));
}

#[test]
fn test_should_skip_file_by_extension() {
    // Skip compiled/data files
    assert!(should_skip_file(Path::new("file.json"), 100));
    assert!(should_skip_file(Path::new("file.min.js"), 100));
    assert!(should_skip_file(Path::new("file.lock"), 100));
    assert!(should_skip_file(Path::new("file.png"), 100));
    assert!(should_skip_file(Path::new("file.pb.go"), 100));
}

#[test]
fn test_should_skip_file_allowed_extensions() {
    // Keep source code files
    assert!(!should_skip_file(Path::new("file.rs"), 100));
    assert!(!should_skip_file(Path::new("file.py"), 100));
    assert!(!should_skip_file(Path::new("file.js"), 100));
    assert!(!should_skip_file(Path::new("file.go"), 100));
    assert!(!should_skip_file(Path::new("file.ts"), 100));
}

#[test]
fn test_should_skip_file_by_size() {
    // Oversized files are skipped
    assert!(should_skip_file(Path::new("file.rs"), 600 * 1024)); // 600 KB
    assert!(!should_skip_file(Path::new("file.rs"), 400 * 1024)); // 400 KB (under limit)
}

#[test]
fn test_should_skip_file_dotfiles() {
    assert!(should_skip_file(Path::new(".env"), 100));
    assert!(should_skip_file(Path::new(".gitignore"), 100));
    assert!(should_skip_file(Path::new(".editorconfig"), 100));
}

#[test]
fn test_parse_cochange_log_empty() {
    let result = parse_cochange_log("");
    assert!(result.is_empty());
}

#[test]
fn test_parse_cochange_log_single_commit() {
    let input = "src/a.rs\nsrc/b.rs\nATLASCOMMIT\n";
    let result = parse_cochange_log(input);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec!["src/a.rs", "src/b.rs"]);
}

#[test]
fn test_parse_cochange_log_multiple_commits() {
    let input = "src/a.rs\nsrc/b.rs\nATLASCOMMIT\nsrc/c.rs\nsrc/d.rs\nATLASCOMMIT\n";
    let result = parse_cochange_log(input);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], vec!["src/a.rs", "src/b.rs"]);
    assert_eq!(result[1], vec!["src/c.rs", "src/d.rs"]);
}

#[test]
fn test_parse_cochange_log_with_whitespace() {
    let input = "  src/a.rs  \n  src/b.rs  \nATLASCOMMIT\n";
    let result = parse_cochange_log(input);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec!["src/a.rs", "src/b.rs"]);
}

#[test]
fn test_parse_cochange_log_skips_large_commits() {
    // Commits with > 50 files are skipped
    let mut input = String::new();
    for i in 0..60 {
        input.push_str(&format!("src/file{}.rs\n", i));
    }
    input.push_str("ATLASCOMMIT\n");

    let result = parse_cochange_log(&input);
    assert!(result.is_empty()); // Entire commit should be skipped
}

#[test]
fn test_parse_cochange_log_preserves_order() {
    let input = "z.rs\na.rs\nm.rs\nATLASCOMMIT\n";
    let result = parse_cochange_log(input);
    assert_eq!(result[0], vec!["z.rs", "a.rs", "m.rs"]); // Order preserved
}

#[test]
fn test_build_cochange_pairs_single_commit() {
    let commits = vec![
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[&("src/a.rs".to_string(), "src/b.rs".to_string())], 1);
}

#[test]
fn test_build_cochange_pairs_repeated_pairs() {
    let commits = vec![
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    assert_eq!(pairs[&("src/a.rs".to_string(), "src/b.rs".to_string())], 3);
}

#[test]
fn test_build_cochange_pairs_all_combinations() {
    let commits = vec![
        vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    // Should have 3 pairs: (a,b), (a,c), (b,c)
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[&("a.rs".to_string(), "b.rs".to_string())], 1);
    assert_eq!(pairs[&("a.rs".to_string(), "c.rs".to_string())], 1);
    assert_eq!(pairs[&("b.rs".to_string(), "c.rs".to_string())], 1);
}

#[test]
fn test_build_cochange_pairs_deduplication() {
    let commits = vec![
        vec!["a.rs".to_string(), "b.rs".to_string(), "a.rs".to_string()], // duplicate a.rs
    ];
    let pairs = build_cochange_pairs(&commits);
    // Should still be 1 pair (a,b), not 2
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[&("a.rs".to_string(), "b.rs".to_string())], 1);
}

#[test]
fn test_build_cochange_pairs_lexicographic_order() {
    let commits = vec![
        vec!["z.rs".to_string(), "a.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    // Pair should be (a.rs, z.rs), not (z.rs, a.rs)
    assert_eq!(pairs.len(), 1);
    assert!(pairs.contains_key(&("a.rs".to_string(), "z.rs".to_string())));
    assert!(!pairs.contains_key(&("z.rs".to_string(), "a.rs".to_string())));
}

#[test]
fn test_build_cochange_pairs_multiple_commits_merge() {
    let commits = vec![
        vec!["a.rs".to_string(), "b.rs".to_string()],
        vec!["a.rs".to_string(), "c.rs".to_string()],
        vec!["a.rs".to_string(), "b.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    // (a,b) should be counted twice, (a,c) once
    assert_eq!(pairs[&("a.rs".to_string(), "b.rs".to_string())], 2);
    assert_eq!(pairs[&("a.rs".to_string(), "c.rs".to_string())], 1);
    // (b,c) is not present since they never appear together
    assert!(!pairs.contains_key(&("b.rs".to_string(), "c.rs".to_string())));
}

#[test]
fn test_build_cochange_pairs_empty() {
    let commits: Vec<Vec<String>> = vec![];
    let pairs = build_cochange_pairs(&commits);
    assert!(pairs.is_empty());
}

#[test]
fn test_build_cochange_pairs_single_file_commits() {
    let commits = vec![
        vec!["a.rs".to_string()],
        vec!["b.rs".to_string()],
    ];
    let pairs = build_cochange_pairs(&commits);
    // No pairs can be formed from single-file commits
    assert!(pairs.is_empty());
}
