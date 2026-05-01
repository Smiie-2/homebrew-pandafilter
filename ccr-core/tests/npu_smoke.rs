//! Feature-gated NPU smoke test.
//!
//! Skipped at compile time unless `--features openvino`; skipped at run time
//! unless `OPENVINO_NPU_AVAILABLE=1`. The point is to verify that the
//! OpenVINO EP can build a session for our default model on the host's NPU
//! and that embeddings come back the right shape and L2-normalised.

#![cfg(feature = "openvino")]

use panda_core::summarizer;

fn npu_opted_in() -> bool {
    std::env::var("OPENVINO_NPU_AVAILABLE").ok().as_deref() == Some("1")
}

#[test]
fn npu_smoke_embeds_three_strings() {
    if !npu_opted_in() {
        eprintln!("skipping: OPENVINO_NPU_AVAILABLE != 1");
        return;
    }
    summarizer::set_execution_provider("npu");
    let texts = vec!["error: build failed", "warning: deprecated", "ok"];
    let embeddings = summarizer::embed_direct(texts).expect("embed_direct");
    assert_eq!(embeddings.len(), 3, "expected 3 vectors");
    for (i, v) in embeddings.iter().enumerate() {
        assert_eq!(v.len(), 384, "vec {i} dim");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "vec {i} not L2-normalised: norm={norm}"
        );
    }
}

#[test]
fn npu_falls_back_to_cpu_when_openvino_missing() {
    if !npu_opted_in() {
        eprintln!("skipping: OPENVINO_NPU_AVAILABLE != 1");
        return;
    }
    // Hide the OV runtime so OpenVINO EP construction fails. The test only
    // works if libopenvino_c.so isn't on a system path the loader will find
    // anyway. On systems where it is, this test is a no-op and that's fine.
    std::env::set_var("LD_LIBRARY_PATH", "/dev/null");
    std::env::remove_var("PANDA_NPU_STRICT");
    summarizer::set_execution_provider("npu");

    // Construction should succeed via CPU fallback.
    let r = summarizer::embed_direct(vec!["x"]);
    assert!(r.is_ok(), "expected CPU fallback to succeed: {:?}", r.err());
}
