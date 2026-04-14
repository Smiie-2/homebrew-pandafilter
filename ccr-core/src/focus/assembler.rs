//! Output assembler — build the guidance block from query results.

use crate::focus::query::RankedFile;
use serde::{Deserialize, Serialize};

/// Guidance output: recommended files, excluded files, negative guidance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuidanceOutput {
    pub recommended_files: Vec<FileEntry>,
    pub negative_guidance: Vec<String>,
    pub guidance_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub role: String,
    pub confidence: f64,
}

/// Assemble guidance from query results.
///
/// Builds a formatted guidance block that tells Claude:
/// 1. Which files to focus on (recommended)
/// 2. Which files to explicitly avoid (negative guidance)
pub fn assemble(
    recommended: Vec<RankedFile>,
    total_files: usize,
) -> GuidanceOutput {
    // Convert to display format
    let recommended_files: Vec<FileEntry> = recommended
        .iter()
        .map(|r| FileEntry {
            path: r.path.clone(),
            role: r.role.clone(),
            confidence: r.confidence,
        })
        .collect();

    // Build negative guidance: identify exclusion patterns
    let negative_guidance = build_negative_guidance(&recommended_files);

    // Format as human-readable text
    let guidance_text = format_guidance(&recommended_files, total_files);

    GuidanceOutput {
        recommended_files,
        negative_guidance,
        guidance_text,
    }
}

/// Build negative guidance: files and patterns to explicitly avoid.
///
/// Returns actionable exclusion messages based on common noise patterns
/// and file roles not represented in the recommendations.
fn build_negative_guidance(recommended: &[FileEntry]) -> Vec<String> {
    if recommended.is_empty() {
        return vec![];
    }

    let mut guidance = Vec::new();

    // Identify which roles are already covered
    let has_tests = recommended.iter().any(|f| f.role == "test");
    let has_persistence = recommended.iter().any(|f| f.role == "persistence");

    // Add exclusion guidance for common noise patterns
    let mut exclusions = Vec::new();

    // Always exclude vendor/dependencies
    exclusions.push("node_modules/, vendor/, .git/ (dependencies)");

    // Suggest skipping test files if not already recommended
    if !has_tests {
        exclusions.push("*_test.rs, *_test.py, test/ (tests)");
    }

    // Suggest skipping old/legacy code if persistence isn't recommended
    if !has_persistence {
        exclusions.push("old/, legacy/, deprecated/ (obsolete code)");
    }

    if !exclusions.is_empty() {
        guidance.push(format!(
            "Skip these file patterns to avoid distraction: {}",
            exclusions.join("; ")
        ));
    }

    guidance
}

/// Format guidance as human-readable text for injection into context.
fn format_guidance(recommended: &[FileEntry], total_files: usize) -> String {
    if recommended.is_empty() {
        return "## Context Focus\nNo specific files identified as most relevant.".to_string();
    }

    let mut text = String::from("## Context Focus\nRelevant files to prioritize:\n");

    for (i, file) in recommended.iter().take(6).enumerate() {
        // Format role for display
        let role_display = match file.role.as_str() {
            "entry_point" => "Entry Point",
            "persistence" => "Database/Storage",
            "state_manager" => "State",
            "validator" => "Validation",
            "test" => "Test",
            _ => "Other",
        };

        text.push_str(&format!(
            "{}. `{}` [{}]\n",
            i + 1,
            file.path,
            role_display
        ));
    }

    let excluded_count = total_files.saturating_sub(recommended.len());
    text.push_str(&format!(
        "\n{} other files in this repo — focus on the above for efficiency.",
        excluded_count
    ));

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_guidance_empty() {
        let text = format_guidance(&[], 50);
        assert!(text.contains("No specific files"));
    }

    #[test]
    fn test_format_guidance_with_files() {
        let files = vec![
            FileEntry {
                path: "src/main.rs".to_string(),
                role: "entry_point".to_string(),
                confidence: 0.95,
            },
            FileEntry {
                path: "src/db.rs".to_string(),
                role: "persistence".to_string(),
                confidence: 0.87,
            },
        ];

        let text = format_guidance(&files, 50);
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("Entry Point"));
        assert!(text.contains("src/db.rs"));
        assert!(text.contains("Database/Storage"));
        assert!(text.contains("48 other files"));
    }

    #[test]
    fn test_assemble_creates_output() {
        let ranked = vec![RankedFile {
            path: "src/main.rs".to_string(),
            role: "entry_point".to_string(),
            confidence: 0.95,
            cochange_count: 5,
            relevance_score: 0.92,
        }];

        let output = assemble(ranked, 100);
        assert_eq!(output.recommended_files.len(), 1);
        assert!(!output.guidance_text.is_empty());
    }
}
