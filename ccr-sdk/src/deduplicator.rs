use std::collections::HashMap;

use panda_core::sentence::split_sentences;
use panda_core::summarizer::embed_batch;

use crate::message::Message;

/// Similarity threshold above which a sentence is considered a duplicate of an
/// earlier sentence in the conversation.
const DEDUP_THRESHOLD: f32 = 0.92;

/// Maximum total sentences to embed across all prior turns combined.
/// The current message is always included in full; oldest prior sentences are dropped first.
/// Rationale: 150 × ~5ms/sentence ≈ 750ms ceiling on modern hardware.
const DEDUP_SENTENCE_CAP: usize = 150;

/// Remove redundant sentences from user messages that restate content already
/// present in an earlier turn.
///
/// For each user message (oldest to newest), sentences that are semantically
/// near-identical (cosine similarity ≥ 0.92) to a sentence in a prior user turn
/// are replaced with `[covered in turn N]`.
///
/// Assistant messages are never modified.
pub fn deduplicate(messages: Vec<Message>) -> Vec<Message> {
    // Collect all user sentences with their message index and sentence index.
    let user_sentence_positions: Vec<(usize, usize, String)> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .flat_map(|(msg_i, m)| {
            split_sentences(&m.content)
                .into_iter()
                .enumerate()
                .map(move |(s_i, s)| (msg_i, s_i, s))
        })
        .collect();

    // Need at least two sentences across at least two messages to dedup anything.
    let unique_msg_indices: std::collections::HashSet<usize> =
        user_sentence_positions.iter().map(|(i, _, _)| *i).collect();
    if unique_msg_indices.len() < 2 {
        return messages;
    }

    // Cap total sentences to avoid unbounded BERT cost as conversations grow.
    // Keep current message whole; trim oldest prior sentences first.
    let user_sentence_positions: Vec<(usize, usize, String)> =
        if user_sentence_positions.len() > DEDUP_SENTENCE_CAP {
            let last_msg = user_sentence_positions
                .last()
                .map(|(i, _, _)| *i)
                .unwrap_or(0);
            let (current, prior): (Vec<_>, Vec<_>) = user_sentence_positions
                .into_iter()
                .partition(|(i, _, _)| *i == last_msg);
            let budget = DEDUP_SENTENCE_CAP.saturating_sub(current.len());
            let prior_len = prior.len();
            let prior_trimmed: Vec<_> = if prior_len > budget {
                prior.into_iter().skip(prior_len - budget).collect()
            } else {
                prior
            };
            let mut merged: Vec<_> = prior_trimmed.into_iter().chain(current).collect();
            merged.sort_by_key(|(i, j, _)| (*i, *j));
            merged
        } else {
            user_sentence_positions
        };

    // Embed all user sentences in one batch for efficiency.
    let texts: Vec<&str> = user_sentence_positions.iter().map(|(_, _, s)| s.as_str()).collect();
    let embeddings = match embed_batch(&texts) {
        Ok(e) => e,
        Err(_) => return messages, // fall back to no-op on embedding failure
    };

    // Build a lookup: flat index → (msg_idx, sentence_idx).
    // For each sentence, find if any earlier message contains a near-duplicate.
    // replacements maps (msg_idx, sentence_idx) → the 1-based user turn number
    // of the earlier message that covers this sentence.
    let mut replacements: HashMap<(usize, usize), usize> = HashMap::new();

    // Map message index → 1-based user turn number (for display).
    let mut user_turn_number: HashMap<usize, usize> = HashMap::new();
    {
        let mut turn = 0usize;
        for (msg_i, _, _) in &user_sentence_positions {
            user_turn_number.entry(*msg_i).or_insert_with(|| {
                turn += 1;
                turn
            });
        }
    }

    for (flat_i, (msg_i, s_i, _)) in user_sentence_positions.iter().enumerate() {
        // Compare against all sentences from strictly older messages.
        for (flat_j, (prev_msg_i, _, _)) in user_sentence_positions[..flat_i].iter().enumerate() {
            if *prev_msg_i >= *msg_i {
                continue;
            }
            let sim = cosine_similarity(&embeddings[flat_i], &embeddings[flat_j]);
            if sim >= DEDUP_THRESHOLD {
                let turn_n = *user_turn_number.get(prev_msg_i).unwrap_or(&1);
                replacements.insert((*msg_i, *s_i), turn_n);
                break;
            }
        }
    }

    if replacements.is_empty() {
        return messages;
    }

    messages
        .into_iter()
        .enumerate()
        .map(|(msg_i, mut msg)| {
            if msg.role != "user" {
                return msg;
            }
            let sentences = split_sentences(&msg.content);
            let new_parts: Vec<String> = sentences
                .into_iter()
                .enumerate()
                .map(|(s_i, s)| {
                    if let Some(&turn_n) = replacements.get(&(msg_i, s_i)) {
                        format!("[covered in turn {}]", turn_n)
                    } else {
                        s
                    }
                })
                .collect();
            msg.content = new_parts.join(" ");
            msg
        })
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // embed_batch returns L2-normalized vectors, so cosine similarity = dot product.
    // Clamped to [-1, 1] to absorb floating-point rounding near unit length.
    let v: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    v.clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message { role: role.to_string(), content: content.to_string() }
    }

    #[test]
    fn passthrough_with_single_user_message() {
        let msgs = vec![
            msg("user", "Hello. How are you?"),
            msg("assistant", "Fine."),
        ];
        let result = deduplicate(msgs.clone());
        assert_eq!(result[0].content, msgs[0].content);
    }

    #[test]
    fn assistant_messages_never_modified() {
        let msgs = vec![
            msg("assistant", "Remember: always use Rust."),
            msg("user", "Ok."),
            msg("assistant", "Remember: always use Rust."),
        ];
        let result = deduplicate(msgs.clone());
        assert_eq!(result[0].content, msgs[0].content);
        assert_eq!(result[2].content, msgs[2].content);
    }

    /// Verify the cap: when total prior sentences exceed DEDUP_SENTENCE_CAP,
    /// the collected texts vector stays within bounds.
    #[test]
    fn sentence_cap_trims_oldest_prior_sentences() {
        // Build 9 messages each with 20 sentences — 180 total, over the cap of 150.
        // The last message is "current"; prior = 8 × 20 = 160 sentences.
        // After capping: current (20) + 130 prior = 150 total.
        let make_msg = |role: &str, n: usize, prefix: &str| {
            // Each sentence is distinct to avoid triggering early-return on unique_msg_indices.
            let content = (0..n)
                .map(|i| format!("{}sentence{i}.", prefix))
                .collect::<Vec<_>>()
                .join(" ");
            msg(role, &content)
        };

        let mut messages = Vec::new();
        for i in 0..9usize {
            messages.push(make_msg("user", 20, &format!("msg{i}_")));
        }
        // We can't easily inspect the internal `texts` vector without refactoring,
        // so we verify the observable property: the function doesn't panic and
        // returns the right number of messages.
        let result = deduplicate(messages.clone());
        assert_eq!(result.len(), messages.len(), "deduplicate must return same number of messages");
    }

    /// Current message sentences must survive even when prior messages fill the cap.
    #[test]
    fn sentence_cap_always_keeps_current_message() {
        // 8 prior messages × 20 sentences = 160 prior sentences.
        // The 9th (current) message has 20 sentences of its own.
        // After cap: current should be fully present.
        let make_msg = |role: &str, n: usize, prefix: &str| {
            let content = (0..n)
                .map(|i| format!("{}sentence{i}.", prefix))
                .collect::<Vec<_>>()
                .join(" ");
            msg(role, &content)
        };

        let mut messages: Vec<_> = (0..8usize)
            .map(|i| make_msg("user", 20, &format!("old{i}_")))
            .collect();
        messages.push(make_msg("user", 20, "current_"));

        // Function should complete without panic.
        let result = deduplicate(messages.clone());
        assert_eq!(result.len(), messages.len());
        // The last (current) message content should not be reduced
        // (no dedup happens since all sentences are unique).
        assert_eq!(result.last().unwrap().content, messages.last().unwrap().content);
    }
}
