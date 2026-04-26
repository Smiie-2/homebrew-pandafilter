//! Embedder-quality bench.
//!
//! Reuses `*.qa.toml` + `*.txt` fixtures: for each question, mark lines that
//! contain any `key_facts` substring as relevant, then ask the active embedder
//! to rank lines against the question. Reports MRR + Hit@k + nDCG@10.
//!
//! No LLM calls, no compression pipeline — just the embedder under test.

use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::runner::{discover_fixtures, QaFixture};

#[derive(Debug, Clone, Serialize)]
pub struct QuestionScore {
    pub question: String,
    pub n_lines: usize,
    pub n_relevant: usize,
    pub mrr: f32,
    pub hit_at_1: f32,
    pub hit_at_5: f32,
    pub ndcg_at_10: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixtureScore {
    pub name: String,
    pub n_questions: usize,
    pub n_skipped: usize,
    pub mrr: f32,
    pub hit_at_1: f32,
    pub hit_at_5: f32,
    pub ndcg_at_10: f32,
    pub questions: Vec<QuestionScore>,
}

#[derive(Debug, Serialize)]
pub struct EmbedReport {
    pub model: String,
    pub mode: String,
    pub n_questions_total: usize,
    pub n_skipped_total: usize,
    pub overall_mrr: f32,
    pub overall_hit_at_1: f32,
    pub overall_hit_at_5: f32,
    pub overall_ndcg_at_10: f32,
    pub fixtures: Vec<FixtureScore>,
}

pub fn run(fixtures_dir: &Path) -> Result<EmbedReport> {
    let pairs = discover_fixtures(fixtures_dir)?;

    let mut fixtures = Vec::new();
    let mut total_q = 0usize;
    let mut total_skipped = 0usize;
    let mut sum_mrr = 0.0f32;
    let mut sum_h1 = 0.0f32;
    let mut sum_h5 = 0.0f32;
    let mut sum_ndcg = 0.0f32;
    let mut counted = 0usize;

    for (txt_path, qa_path) in pairs {
        let name = txt_path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&txt_path)?;
        let qa: QaFixture = toml::from_str(&std::fs::read_to_string(&qa_path)?)?;

        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.trim().is_empty())
            .collect();

        let mut q_scores = Vec::new();
        let mut skipped_in_fixture = 0usize;

        for q in &qa.questions {
            let relevant: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| is_relevant(line, &q.key_facts))
                .map(|(i, _)| i)
                .collect();

            if relevant.is_empty() || relevant.len() == lines.len() {
                skipped_in_fixture += 1;
                continue;
            }

            let score = score_question(&q.question, &lines, &relevant)?;
            q_scores.push(score);
        }

        let n_q = q_scores.len();
        let mrr = mean(&q_scores, |s| s.mrr);
        let h1 = mean(&q_scores, |s| s.hit_at_1);
        let h5 = mean(&q_scores, |s| s.hit_at_5);
        let ndcg = mean(&q_scores, |s| s.ndcg_at_10);

        total_q += qa.questions.len();
        total_skipped += skipped_in_fixture;
        if n_q > 0 {
            sum_mrr += mrr * n_q as f32;
            sum_h1 += h1 * n_q as f32;
            sum_h5 += h5 * n_q as f32;
            sum_ndcg += ndcg * n_q as f32;
            counted += n_q;
        }

        fixtures.push(FixtureScore {
            name,
            n_questions: qa.questions.len(),
            n_skipped: skipped_in_fixture,
            mrr,
            hit_at_1: h1,
            hit_at_5: h5,
            ndcg_at_10: ndcg,
            questions: q_scores,
        });
    }

    let denom = counted.max(1) as f32;
    Ok(EmbedReport {
        model: panda_core::summarizer::current_model_name().to_string(),
        mode: std::env::var("PANDA_NPU").unwrap_or_else(|_| "auto".into()),
        n_questions_total: total_q,
        n_skipped_total: total_skipped,
        overall_mrr: sum_mrr / denom,
        overall_hit_at_1: sum_h1 / denom,
        overall_hit_at_5: sum_h5 / denom,
        overall_ndcg_at_10: sum_ndcg / denom,
        fixtures,
    })
}

fn is_relevant(line: &str, key_facts: &[String]) -> bool {
    let lower = line.to_lowercase();
    key_facts.iter().any(|f| lower.contains(&f.to_lowercase()))
}

fn score_question(query: &str, lines: &[&str], relevant: &[usize]) -> Result<QuestionScore> {
    let mut batch: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    batch.push(query);
    batch.extend_from_slice(lines);

    let embeddings = panda_core::summarizer::embed_batch(&batch)?;
    let q_emb = &embeddings[0];

    let mut scored: Vec<(usize, f32)> = (0..lines.len())
        .map(|i| (i, dot(q_emb, &embeddings[i + 1])))
        .collect();
    scored.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let relevant_set: std::collections::HashSet<usize> = relevant.iter().copied().collect();

    let mut first_hit_rank: Option<usize> = None;
    let mut hit_at_1 = 0.0f32;
    let mut hit_at_5 = 0.0f32;
    let mut dcg = 0.0f32;
    for (rank0, (idx, _)) in scored.iter().enumerate() {
        let rank = rank0 + 1;
        let is_rel = relevant_set.contains(idx);
        if is_rel && first_hit_rank.is_none() {
            first_hit_rank = Some(rank);
        }
        if rank == 1 && is_rel {
            hit_at_1 = 1.0;
        }
        if rank <= 5 && is_rel {
            hit_at_5 = 1.0;
        }
        if rank <= 10 && is_rel {
            dcg += 1.0 / ((rank as f32 + 1.0).log2());
        }
    }

    let ideal_hits = relevant.len().min(10);
    let idcg: f32 = (1..=ideal_hits)
        .map(|r| 1.0 / ((r as f32 + 1.0).log2()))
        .sum();
    let ndcg = if idcg > 0.0 { dcg / idcg } else { 0.0 };

    let mrr = first_hit_rank.map(|r| 1.0 / r as f32).unwrap_or(0.0);

    Ok(QuestionScore {
        question: query.to_string(),
        n_lines: lines.len(),
        n_relevant: relevant.len(),
        mrr,
        hit_at_1,
        hit_at_5,
        ndcg_at_10: ndcg,
    })
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn mean<T, F: Fn(&T) -> f32>(items: &[T], f: F) -> f32 {
    if items.is_empty() {
        0.0
    } else {
        items.iter().map(f).sum::<f32>() / items.len() as f32
    }
}

pub fn print_and_save(report: &EmbedReport, bench_dir: &Path) -> Result<()> {
    println!();
    println!(
        "Embedder Quality Bench — model={} ({})",
        report.model, report.mode
    );
    println!("{}", "═".repeat(85));
    println!(
        "{:<20} {:>9} {:>8} {:>7} {:>7} {:>7} {:>9}",
        "Fixture", "Questions", "Skipped", "MRR", "Hit@1", "Hit@5", "nDCG@10"
    );
    println!("{}", "─".repeat(85));
    for f in &report.fixtures {
        println!(
            "{:<20} {:>9} {:>8} {:>7.3} {:>7.2} {:>7.2} {:>9.3}",
            truncate(&f.name, 20),
            f.n_questions,
            f.n_skipped,
            f.mrr,
            f.hit_at_1,
            f.hit_at_5,
            f.ndcg_at_10
        );
    }
    println!("{}", "─".repeat(85));
    println!(
        "{:<20} {:>9} {:>8} {:>7.3} {:>7.2} {:>7.2} {:>9.3}",
        "OVERALL",
        report.n_questions_total,
        report.n_skipped_total,
        report.overall_mrr,
        report.overall_hit_at_1,
        report.overall_hit_at_5,
        report.overall_ndcg_at_10,
    );
    println!();

    let reports_dir = bench_dir.join("reports");
    std::fs::create_dir_all(&reports_dir)?;
    let safe_model = report.model.replace('/', "_");
    let json_path = reports_dir.join(format!("embed_{}.json", safe_model));
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&json_path, json)?;
    println!("JSON report → {}", json_path.display());
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}

