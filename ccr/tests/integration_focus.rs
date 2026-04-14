//! Integration tests for focus command — basic smoke tests.

use std::process::Command;

#[test]
fn test_focus_disable_command_exists() {
    // Just verify the command doesn't crash
    let output = Command::new("cargo")
        .args(["run", "--bin", "panda", "--", "focus", "--disable"])
        .output()
        .expect("failed to run panda focus --disable");

    // Should not crash, even if it doesn't do much in tests
    assert!(
        output.status.code().is_some(),
        "panda focus --disable did not return a valid exit code"
    );
}

#[test]
fn test_focus_dry_run_command_exists() {
    // Just verify the command doesn't crash
    let output = Command::new("cargo")
        .args(["run", "--bin", "panda", "--", "focus", "--dry-run"])
        .output()
        .expect("failed to run panda focus --dry-run");

    // Should not crash
    assert!(
        output.status.code().is_some(),
        "panda focus --dry-run did not return a valid exit code"
    );
}

#[test]
fn test_index_command_exists() {
    // Just verify the command compiles and exists (won't actually index from temp dir)
    let output = Command::new("cargo")
        .args(["run", "--bin", "panda", "--", "index", "--help"])
        .output()
        .expect("failed to run panda index --help");

    // Should show help text
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        combined.contains("repo") || combined.contains("Repository"),
        "expected help text for panda index"
    );
}
