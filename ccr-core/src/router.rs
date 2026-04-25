//! MoE-inspired sparse filter router.
//!
//! Analyzes content features in a single O(n) pass, scores 8 filter "experts",
//! and returns the top-K to activate. This replaces the dense pipeline (all stages
//! always) with sparse routing — only the most relevant experts fire per input.
//!
//! Gated behind `[global] use_router = true` in `panda.toml` (default: off).

// ── Expert IDs ────────────────────────────────────────────────────────────────

/// The set of available filter experts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpertId {
    /// Keep only error/warning/failure lines + N-line context.
    ErrorFocus,
    /// Extract stats/summaries (counts, percentages, headers).
    StatExtract,
    /// Simhash-based near-dedup with repetition collapsing.
    Dedup,
    /// Diff against cached prior read (delta mode).
    DeltaMode,
    /// Structural digest (function/class signatures only).
    StructureOnly,
    /// Full BERT semantic summarization pipeline.
    SemanticSummary,
    /// Hierarchy compression with file counts (ls -R, find, tree).
    TreeCompress,
    /// No filtering — pass through unchanged.
    PassThrough,
}

impl ExpertId {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ErrorFocus => "error-focus",
            Self::StatExtract => "stat-extract",
            Self::Dedup => "dedup",
            Self::DeltaMode => "delta-mode",
            Self::StructureOnly => "structure-only",
            Self::SemanticSummary => "semantic-summary",
            Self::TreeCompress => "tree-compress",
            Self::PassThrough => "pass-through",
        }
    }

    pub const ALL: [ExpertId; 8] = [
        ExpertId::ErrorFocus,
        ExpertId::StatExtract,
        ExpertId::Dedup,
        ExpertId::DeltaMode,
        ExpertId::StructureOnly,
        ExpertId::SemanticSummary,
        ExpertId::TreeCompress,
        ExpertId::PassThrough,
    ];
}

// ── Content features ──────────────────────────────────────────────────────────

/// Cheap O(1-pass) features extracted from content + session context.
/// All fields are in [0.0, 1.0] unless otherwise noted.
#[derive(Debug, Clone)]
pub struct ContentFeatures {
    /// Total line count (raw count, not normalized).
    pub line_count: usize,
    /// Shannon entropy estimate: 0 = uniform/repetitive, 1 = high-entropy.
    pub entropy: f32,
    /// Fraction of lines matching error/warn/fail/fatal/panic/exception.
    pub error_line_density: f32,
    /// Fraction of lines that are near-duplicates (simhash collision estimate).
    pub repetition_ratio: f32,
    /// True if first non-empty line parses as JSON object.
    pub is_json: bool,
    /// True if output looks like a directory tree (ls -R, find, tree).
    pub is_tree: bool,
    /// True if content contains stack trace markers (at , File , goroutine , panic).
    pub has_stack_trace: bool,
    /// Average characters per line.
    pub avg_line_length: f32,
    /// Session context pressure (0 = relaxed, 1 = full).
    pub session_pressure: f32,
    /// True if this file was read earlier in the current session.
    pub is_recent_read: bool,
    /// True if mtime has not changed since last read (content unchanged).
    pub is_unchanged_read: bool,
}

impl Default for ContentFeatures {
    fn default() -> Self {
        Self {
            line_count: 0,
            entropy: 0.5,
            error_line_density: 0.0,
            repetition_ratio: 0.0,
            is_json: false,
            is_tree: false,
            has_stack_trace: false,
            avg_line_length: 60.0,
            session_pressure: 0.0,
            is_recent_read: false,
            is_unchanged_read: false,
        }
    }
}

/// Extract content features in a single O(n) pass through the input.
pub fn extract_features(content: &str) -> ContentFeatures {
    let mut f = ContentFeatures::default();

    let lines: Vec<&str> = content.lines().collect();
    f.line_count = lines.len();
    if f.line_count == 0 {
        return f;
    }

    // --- Single pass ---
    let mut total_chars: usize = 0;
    let mut error_lines: usize = 0;
    let mut char_freq = [0u64; 256];
    let mut has_indent = false;
    let mut tree_markers = 0usize;
    let mut stack_markers = 0usize;
    // Near-dup: sample first 200 lines and count hash collisions
    let mut seen_hashes = std::collections::HashSet::new();
    let mut dup_count: usize = 0;
    let sample_limit = lines.len().min(200);

    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim();
        total_chars += line.len();

        // Char frequency for entropy
        for &b in line.as_bytes().iter().take(128) {
            char_freq[b as usize] += 1;
        }

        // Error density
        let tl = t.to_ascii_lowercase();
        if tl.contains("error") || tl.contains("warn") || tl.contains("fail")
            || tl.contains("fatal") || tl.contains("panic") || tl.contains("exception")
        {
            error_lines += 1;
        }

        // Tree markers: lines starting with ├, └, │, or matching ls -R pattern
        if t.starts_with('\u{251C}') || t.starts_with('\u{2514}') || t.starts_with('\u{2502}')
            || (t.ends_with(':') && !t.contains(' ') && t.contains('/'))
        {
            tree_markers += 1;
        }

        // Stack trace: " at " (JS/Java), "File " (Python), "goroutine " (Go), "\tat " (Java)
        if t.starts_with("at ") || t.contains(" at ") || t.starts_with("File \"")
            || t.starts_with("goroutine ") || t.starts_with("\tat ")
        {
            stack_markers += 1;
        }

        // Indentation check (rough proxy for structured code)
        if line.starts_with("    ") || line.starts_with('\t') {
            has_indent = true;
        }

        // Near-dup sampling
        if idx < sample_limit {
            let h = fast_hash(t);
            if !seen_hashes.insert(h) {
                dup_count += 1;
            }
        }
    }

    f.avg_line_length = total_chars as f32 / f.line_count as f32;
    f.error_line_density = error_lines as f32 / f.line_count as f32;
    f.repetition_ratio = dup_count as f32 / sample_limit as f32;
    f.has_stack_trace = stack_markers >= 3;
    f.is_tree = tree_markers as f32 / f.line_count as f32 > 0.15;

    // Shannon entropy from char frequency
    let total_bytes: u64 = char_freq.iter().sum();
    if total_bytes > 0 {
        let mut entropy = 0.0f64;
        for &c in char_freq.iter() {
            if c > 0 {
                let p = c as f64 / total_bytes as f64;
                entropy -= p * p.log2();
            }
        }
        // Max entropy for ASCII ≈ 8 bits; normalize to [0,1]
        f.entropy = (entropy / 8.0).clamp(0.0, 1.0) as f32;
    }

    // JSON detection: first non-empty line starts with `{` or `[`
    if let Some(first) = lines.iter().find(|l| !l.trim().is_empty()) {
        let t = first.trim();
        f.is_json = t.starts_with('{') || t.starts_with('[');
    }

    let _ = has_indent; // may be used in future scoring
    f
}

// ── Expert scoring ────────────────────────────────────────────────────────────

/// Score all experts for the given features. Returns scores in ExpertId::ALL order.
/// Higher score = more relevant expert.
pub fn score_experts(f: &ContentFeatures) -> [f32; 8] {
    let mut scores = [0.0f32; 8];

    // ErrorFocus: wins on high error density + stack traces
    scores[0] = f.error_line_density * 2.0
        + if f.has_stack_trace { 1.5 } else { 0.0 };

    // StatExtract: uniform line length, low errors, some structure
    scores[1] = if f.error_line_density < 0.05 && f.avg_line_length < 80.0 {
        0.8 * (1.0 - f.entropy)
    } else {
        0.0
    };

    // Dedup: high repetition ratio wins
    scores[2] = f.repetition_ratio * 2.5;

    // DeltaMode: recent read with changed content
    scores[3] = if f.is_recent_read && !f.is_unchanged_read { 3.0 } else { 0.0 };

    // StructureOnly: unchanged content or very large file
    scores[4] = if f.is_unchanged_read {
        3.0
    } else if f.line_count > 500 {
        1.0
    } else {
        0.0
    };

    // SemanticSummary: high entropy, low repetition
    scores[5] = f.entropy * 1.5 * (1.0 - f.repetition_ratio);

    // TreeCompress: clear tree output
    scores[6] = if f.is_tree { 3.0 } else { 0.0 };

    // PassThrough: small input or high uncertainty
    scores[7] = if f.line_count < 30 {
        2.0
    } else if f.entropy > 0.85 && f.error_line_density < 0.02 && !f.is_tree {
        // High entropy, no errors, not a tree — might be binary/encoded, don't mangle
        1.5
    } else {
        0.0
    };

    scores
}

/// Select the top-K experts using Mixtral-style sparse selection.
/// Returns `(ExpertId, normalized_weight)` pairs, sorted by weight descending.
///
/// K = 2 normally, 3 when `session_pressure > 0.7`.
pub fn top_k_sparse(
    scores: &[f32; 8],
    session_pressure: f32,
    exploration_noise: bool,
    exploration_bonus: Option<&[f32; 8]>,
) -> Vec<(ExpertId, f32)> {
    let k = if session_pressure > 0.7 { 3 } else { 2 };

    let mut adjusted = *scores;

    // Apply exploration noise bonus if provided
    if exploration_noise {
        if let Some(bonus) = exploration_bonus {
            for (i, b) in bonus.iter().enumerate() {
                adjusted[i] += b;
            }
        }
    }

    // Find top-K indices by score
    let mut indexed: Vec<(usize, f32)> = adjusted
        .iter()
        .copied()
        .enumerate()
        .collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.truncate(k);

    // Normalize weights using softmax over selected scores
    let max_score = indexed.iter().map(|(_, s)| *s).fold(f32::NEG_INFINITY, f32::max);
    let exp_scores: Vec<f32> = indexed
        .iter()
        .map(|(_, s)| (*s - max_score).exp())
        .collect();
    let sum_exp: f32 = exp_scores.iter().sum();

    indexed
        .into_iter()
        .zip(exp_scores.into_iter())
        .map(|((idx, _), exp)| {
            let weight = if sum_exp > 0.0 { exp / sum_exp } else { 1.0 / k as f32 };
            (ExpertId::ALL[idx], weight)
        })
        .collect()
}

// ── Expert collapse detection ─────────────────────────────────────────────────

/// Compute Herfindahl-Hirschman Index over expert activation counts.
/// HHI = sum(share^2), range [1/N, 1.0]. High HHI = one expert dominates.
pub fn compute_hhi(activations: &[u64; 8]) -> f32 {
    let total: u64 = activations.iter().sum();
    if total == 0 {
        return 1.0 / 8.0;
    }
    activations
        .iter()
        .map(|&c| (c as f64 / total as f64).powi(2) as f32)
        .sum()
}

/// Compute per-expert exploration bonus when collapse is detected.
/// Returns `Some(bonuses)` if any expert exceeds 70% share, else `None`.
pub fn exploration_bonus(activations: &[u64; 8]) -> Option<[f32; 8]> {
    let total: u64 = activations.iter().sum();
    if total == 0 {
        return None;
    }
    // Find dominating expert
    let max_idx = activations
        .iter()
        .enumerate()
        .max_by_key(|(_, &c)| c)
        .map(|(i, _)| i)?;
    let share = activations[max_idx] as f64 / total as f64;
    if share <= 0.70 {
        return None;
    }
    // All other experts get +0.5 bonus
    let mut bonus = [0.0f32; 8];
    for i in 0..8 {
        if i != max_idx {
            bonus[i] = 0.5;
        }
    }
    Some(bonus)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Fast non-cryptographic hash for near-dup detection.
fn fast_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    h
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_error_heavy() -> String {
        // 60 lines, ~67% error lines — high enough to beat SemanticSummary
        (0..60)
            .map(|i| {
                if i % 3 != 0 {
                    format!("ERROR: something failed on step {}", i)
                } else {
                    format!("running test {}", i)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn make_repetitive() -> String {
        (0..60)
            .map(|_| "2024-01-01T00:00:00Z INFO daemon: heartbeat ok")
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn make_prose() -> String {
        // Generate varied, non-repetitive prose — unique line per index
        let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot",
                     "golf", "hotel", "india", "juliet", "kilo", "lima",
                     "mike", "november", "oscar", "papa", "quebec", "romeo"];
        (0..60)
            .map(|i| format!("{}. {} {} {} {} {} {}",
                i + 1,
                words[i % words.len()],
                words[(i + 3) % words.len()],
                words[(i + 7) % words.len()],
                words[(i + 11) % words.len()],
                words[(i + 13) % words.len()],
                words[(i + 17) % words.len()],
            ))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn error_heavy_routes_to_error_focus() {
        let content = make_error_heavy();
        let features = extract_features(&content);
        let scores = score_experts(&features);
        let top = top_k_sparse(&scores, 0.0, false, None);
        assert_eq!(top[0].0, ExpertId::ErrorFocus, "expected ErrorFocus first, got {:?}", top[0].0);
    }

    #[test]
    fn repetitive_routes_to_dedup() {
        let content = make_repetitive();
        let features = extract_features(&content);
        let scores = score_experts(&features);
        // Dedup score should be high
        assert!(scores[2] > scores[5], "Dedup score should beat SemanticSummary for repetitive input");
    }

    #[test]
    fn prose_routes_to_semantic() {
        let content = make_prose();
        let features = extract_features(&content);
        let scores = score_experts(&features);
        // SemanticSummary should beat Dedup for non-repetitive prose
        assert!(scores[5] > scores[2], "SemanticSummary should beat Dedup for varied prose");
    }

    #[test]
    fn tree_output_routes_to_tree_compress() {
        let content = "\u{251C}\u{2500}\u{2500} src/\n\
                       \u{2502}   \u{251C}\u{2500}\u{2500} main.rs\n\
                       \u{2502}   \u{2514}\u{2500}\u{2500} lib.rs\n\
                       \u{2514}\u{2500}\u{2500} Cargo.toml\n"
            .repeat(10);
        let features = extract_features(&content);
        let scores = score_experts(&features);
        assert!(scores[6] > 2.0, "TreeCompress should score high for tree output");
    }

    #[test]
    fn small_input_passes_through() {
        let content = "hello world\nthis is a small input";
        let features = extract_features(content);
        let scores = score_experts(&features);
        assert!(scores[7] > 0.0, "PassThrough should score >0 for tiny input");
    }

    #[test]
    fn recent_read_routes_to_delta() {
        let features = ContentFeatures {
            is_recent_read: true,
            is_unchanged_read: false,
            line_count: 100,
            ..Default::default()
        };
        let scores = score_experts(&features);
        assert_eq!(scores[3], 3.0, "DeltaMode should score 3.0 for recent changed read");
    }

    #[test]
    fn unchanged_read_routes_to_structure_only() {
        let features = ContentFeatures {
            is_recent_read: true,
            is_unchanged_read: true,
            line_count: 200,
            ..Default::default()
        };
        let scores = score_experts(&features);
        assert_eq!(scores[4], 3.0, "StructureOnly should score 3.0 for unchanged read");
    }

    #[test]
    fn pressure_increases_k() {
        let scores = [0.5f32; 8];
        let low_k = top_k_sparse(&scores, 0.5, false, None);
        let high_k = top_k_sparse(&scores, 0.8, false, None);
        assert_eq!(low_k.len(), 2, "low pressure should give k=2");
        assert_eq!(high_k.len(), 3, "high pressure should give k=3");
    }

    #[test]
    fn top_k_weights_sum_to_one() {
        let scores = [1.0, 2.0, 3.0, 0.5, 0.1, 2.5, 0.0, 1.5];
        let top = top_k_sparse(&scores, 0.0, false, None);
        let sum: f32 = top.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-5, "weights should sum to 1.0, got {sum}");
    }

    #[test]
    fn exploration_bonus_triggers_on_collapse() {
        let mut activations = [0u64; 8];
        activations[5] = 90; // SemanticSummary dominates at 90%
        activations[0] = 10;
        let bonus = exploration_bonus(&activations);
        assert!(bonus.is_some(), "should trigger when one expert > 70%");
        let b = bonus.unwrap();
        assert_eq!(b[5], 0.0, "dominant expert gets no bonus");
        assert_eq!(b[0], 0.5, "other experts get +0.5");
    }

    #[test]
    fn no_collapse_below_threshold() {
        let mut activations = [0u64; 8];
        activations[5] = 60;
        activations[0] = 40;
        let bonus = exploration_bonus(&activations);
        assert!(bonus.is_none(), "should not trigger when no expert > 70%");
    }

    #[test]
    fn hhi_uniform_is_low() {
        let activations = [10u64; 8];
        let hhi = compute_hhi(&activations);
        assert!((hhi - 0.125).abs() < 0.01, "uniform HHI should be 1/8 = 0.125, got {hhi}");
    }

    #[test]
    fn score_experts_no_nan_on_zero_input() {
        let features = ContentFeatures {
            line_count: 0,
            ..Default::default()
        };
        let scores = score_experts(&features);
        for (i, s) in scores.iter().enumerate() {
            assert!(!s.is_nan(), "score[{i}] should not be NaN");
        }
    }
}
