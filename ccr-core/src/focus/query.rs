//! Query module — rank files by relevance using embeddings and cochanges.

use anyhow::Result;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct RankedFile {
    pub path: String,
    pub role: String,
    pub confidence: f64,
    pub cochange_count: i64,
    pub relevance_score: f64,
}

/// Query the focus graph for relevant files given a prompt embedding.
///
/// Returns files ranked by a combination of:
/// 1. Semantic similarity (embedding distance)
/// 2. Co-change frequency (how often changed with similar files)
/// 3. Role classification (entry points scored higher)
pub fn query(
    conn: &Connection,
    prompt_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<RankedFile>> {

    // Get all files with their embeddings
    let mut stmt = conn.prepare(
        "SELECT path, role, role_confidence, embedding, commit_count FROM files"
    )?;

    let mut candidates = Vec::new();
    let rows = stmt.query_map([], |row| {
        let path: String = row.get(0)?;
        let role: String = row.get(1)?;
        let confidence: f64 = row.get(2)?;
        let blob: Vec<u8> = row.get(3)?;
        Ok((path, role, confidence, blob))
    })?;

    for row_result in rows {
        let (path, role, confidence, blob) = row_result?;

        // Reconstruct embedding from blob (4-byte floats)
        let file_embedding = blob_to_embedding(&blob);

        // Compute cosine similarity
        let similarity = cosine_similarity(&prompt_embedding, &file_embedding);

        // Get cochange context (files that co-change with this file)
        let cochange_score = get_cochange_score(conn, &path)?;

        // Compute final relevance score
        let relevance_score = similarity * 0.7 + (cochange_score as f64 * 0.3);

        // Boost entry points
        let role_boost = match role.as_str() {
            "entry_point" => 1.5,
            "persistence" => 1.2,
            "state_manager" => 1.1,
            _ => 1.0,
        };

        let final_score = relevance_score * role_boost;

        candidates.push((path, role, confidence, cochange_score, final_score));
    }

    // Sort by relevance score descending
    candidates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

    // Return top-K
    Ok(candidates
        .into_iter()
        .take(top_k)
        .map(|(path, role, confidence, cochange_count, relevance_score)| {
            RankedFile {
                path,
                role,
                confidence,
                cochange_count,
                relevance_score,
            }
        })
        .collect())
}

/// Get cochange score for a file (sum of all co-occurrence counts)
fn get_cochange_score(conn: &Connection, file_path: &str) -> Result<i64> {
    let score: i64 = conn.query_row(
        "SELECT COALESCE(SUM(change_count), 0) FROM cochanges
         WHERE file_a = ?1 OR file_b = ?1",
        [file_path],
        |row| row.get(0),
    )?;
    Ok(score)
}

/// Convert 4-byte blob to embedding vector
fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks(4)
        .map(|chunk| {
            if chunk.len() == 4 {
                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
            } else {
                0.0
            }
        })
        .collect()
}

/// Compute cosine similarity between two embeddings
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let min_len = a.len().min(b.len());
    let a = &a[..min_len];
    let b = &b[..min_len];

    let dot_product: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();

    let a_norm: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let b_norm: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();

    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }

    dot_product / (a_norm * b_norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&v, &v);
        assert!((similarity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let similarity = cosine_similarity(&a, &b);
        assert!(similarity.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_blob_to_embedding() {
        let bytes = vec![0, 0, 128, 63]; // 1.0 in little-endian f32
        let embedding = blob_to_embedding(&bytes);
        assert_eq!(embedding.len(), 1);
        assert!((embedding[0] - 1.0).abs() < 1e-6);
    }
}
