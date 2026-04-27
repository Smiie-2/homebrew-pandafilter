use std::io::Read;
use std::time::Instant;

fn main() {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let model = std::env::var("PANDA_BERT_MODEL").unwrap_or_else(|_| "SnowflakeArcticEmbedMV2".into());
    let mode = std::env::var("PANDA_NPU").unwrap_or_else(|_| "auto".into());
    let runs: usize = std::env::var("PANDA_BENCH_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    panda_core::summarizer::set_model_name(&model);
    panda_core::summarizer::set_execution_mode(&mode);

    let warm = Instant::now();
    let _ = panda_core::summarizer::summarize(&text, 60);
    let warm_ms = warm.elapsed().as_millis();

    let t0 = Instant::now();
    for _ in 0..runs {
        let _ = panda_core::summarizer::summarize(&text, 60);
    }
    let total = t0.elapsed();
    let avg_us = total.as_micros() / runs as u128;

    println!(
        "{:<24} mode={:<5} warm={:>4}ms  avg={:>5}us  ({:>4}ms/{}runs)",
        model,
        mode,
        warm_ms,
        avg_us,
        total.as_millis(),
        runs
    );
}
