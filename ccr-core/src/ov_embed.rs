use anyhow::{Context, Result};
use openvino::{Core, DeviceType, ElementType, PartialShape, Shape, Tensor};
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

// openvino-sys loads `libopenvino_c.so` (the C API shim), not `libopenvino.so`.
const OV_C_LIB_CANDIDATES: &[&str] = &[
    "/usr/lib/x86_64-linux-gnu/libopenvino_c.so",
    "/usr/local/lib/libopenvino_c.so",
    "/opt/intel/openvino/runtime/lib/intel64/libopenvino_c.so",
];

fn ov_lib_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OPENVINO_LIB_PATH") {
        let pb = PathBuf::from(&p);
        let candidate = if pb.is_dir() {
            pb.join("libopenvino_c.so")
        } else {
            pb
        };
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let ccr_path = std::env::var("HOME").ok().map(|h| {
        PathBuf::from(h).join(".local/share/ccr/onnxruntime/libopenvino_c.so")
    });
    if let Some(p) = ccr_path {
        if p.exists() {
            return Some(p);
        }
    }
    for cand in OV_C_LIB_CANDIDATES {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Per-model mapping: (HF repo, ONNX subpath relative to snapshot root, seq_len)
pub fn model_onnx_info(model_name: &str) -> Option<(&'static str, &'static str, usize)> {
    match model_name {
        "AllMiniLML6V2" =>
            Some(("Qdrant/all-MiniLM-L6-v2-onnx", "model.onnx", 128)),
        "AllMiniLML12V2" =>
            Some(("Xenova/all-MiniLM-L12-v2", "onnx/model.onnx", 128)),
        "BGESmallENV15" =>
            Some(("Xenova/bge-small-en-v1.5", "onnx/model.onnx", 512)),
        "BGESmallENV15Q" =>
            Some(("Qdrant/bge-small-en-v1.5-onnx-Q", "model_optimized.onnx", 512)),
        "MxbaiEmbedLargeV1" =>
            Some(("mixedbread-ai/mxbai-embed-large-v1", "onnx/model.onnx", 512)),
        "MxbaiEmbedLargeV1Q" =>
            Some(("mixedbread-ai/mxbai-embed-large-v1", "onnx/model_quantized.onnx", 512)),
        _ => None,
    }
}

/// Locate the ONNX file and tokenizer.json for `model_name` inside fastembed's cache.
/// Returns `(onnx_path, tokenizer_path, seq_len)`.
pub fn find_fastembed_onnx(
    model_name: &str,
    cache_dir: &Path,
) -> Option<(PathBuf, PathBuf, usize)> {
    let (hf_path, onnx_subpath, seq_len) = model_onnx_info(model_name)?;
    let vendor_model = hf_path.replace('/', "--");
    let snapshots_dir = cache_dir
        .join(format!("models--{}", vendor_model))
        .join("snapshots");
    let hash_dir = std::fs::read_dir(&snapshots_dir)
        .ok()?
        .find_map(|e| e.ok().filter(|e| e.file_type().ok().is_some_and(|t| t.is_dir())).map(|e| e.path()))?;
    let onnx_path = hash_dir.join(onnx_subpath);
    let tokenizer_path = hash_dir.join("tokenizer.json");
    if onnx_path.exists() && tokenizer_path.exists() {
        Some((onnx_path, tokenizer_path, seq_len))
    } else {
        None
    }
}

pub struct OvEmbedder {
    _core: Core,
    compiled: std::sync::Mutex<openvino::CompiledModel>,
    tokenizer: Tokenizer,
    seq_len: usize,
    hidden_size: usize,
}

// SAFETY: OpenVINO documents CompiledModel as thread-safe.
// InferRequest is created per-call and never shared.
unsafe impl Send for OvEmbedder {}
unsafe impl Sync for OvEmbedder {}

impl OvEmbedder {
    pub fn try_new(onnx_path: &Path, tokenizer_path: &Path, seq_len: usize) -> Result<Self> {
        let lib = ov_lib_path()
            .context("libopenvino.so not found; install OpenVINO or set OPENVINO_LIB_PATH")?;
        openvino_sys::library::load_from(&lib)
            .map_err(|e| anyhow::anyhow!("OpenVINO load failed: {e}"))?;

        let mut core = Core::new().context("OpenVINO Core::new() failed")?;

        let mut model = core
            .read_model_from_file(onnx_path.to_str().unwrap(), "")
            .context("Failed to read ONNX model into OpenVINO")?;

        // Reshape all inputs to static [1, seq_len] so the NPU can compile them.
        let n_inputs = model.get_inputs_len()?;
        let static_shape = PartialShape::new_static(2, &[1, seq_len as i64])?;
        for i in 0..n_inputs {
            let name = model.get_input_by_index(i)?.get_name()?;
            model.reshape_input_by_name(&name, &static_shape)?;
        }

        eprint!("[panda] compiling model for NPU (one-time per session)...");
        let compiled = core
            .compile_model(&model, DeviceType::NPU)
            .context("NPU compile_model failed")?;
        eprintln!(" done");

        // Infer hidden_size from last dimension of output 0
        let out_node = compiled.get_output_by_index(0)?;
        let shape = out_node.get_shape()?;
        let dims = shape.get_dimensions();
        let hidden_size = dims.last().copied().unwrap_or(384) as usize;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;

        Ok(OvEmbedder {
            _core: core,
            compiled: std::sync::Mutex::new(compiled),
            tokenizer,
            seq_len,
            hidden_size,
        })
    }

    /// Embed texts and return L2-normalised vectors (one per input).
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_one(t)).collect()
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

        let ids = enc.get_ids();
        let mask = enc.get_attention_mask();
        let actual_len = ids.len().min(self.seq_len);

        let mut id_data = vec![0i64; self.seq_len];
        let mut mask_data = vec![0i64; self.seq_len];
        for k in 0..actual_len {
            id_data[k] = ids[k] as i64;
            mask_data[k] = mask[k] as i64;
        }

        let shape = Shape::new(&[1, self.seq_len as i64])?;
        let mut id_tensor = Tensor::new(ElementType::I64, &shape)?;
        id_tensor.get_data_mut::<i64>()?.copy_from_slice(&id_data);
        let mut mask_tensor = Tensor::new(ElementType::I64, &shape)?;
        mask_tensor.get_data_mut::<i64>()?.copy_from_slice(&mask_data);

        let mut compiled = self.compiled.lock().unwrap();
        let mut req = compiled.create_infer_request()?;
        req.set_tensor("input_ids", &id_tensor)?;
        req.set_tensor("attention_mask", &mask_tensor)?;

        // token_type_ids is optional — ignore errors if the model doesn't have it
        if compiled.get_input_size()? > 2 {
            let tt_tensor = Tensor::new(ElementType::I64, &shape)?;
            let _ = req.set_tensor("token_type_ids", &tt_tensor);
        }
        drop(compiled); // unlock before infer so it's not held during potentially slow inference

        req.infer()?;

        // Retrieve last_hidden_state [1, seq_len, hidden_size] — output 0
        let out = req.get_output_tensor_by_index(0)?;
        let data: &[f32] = out.get_data::<f32>()?;

        // Mean-pool over non-padding positions then L2-normalize
        let mut pooled = vec![0.0f32; self.hidden_size];
        let mut count = 0usize;
        for token_idx in 0..actual_len {
            if mask_data[token_idx] == 0 {
                continue;
            }
            let offset = token_idx * self.hidden_size;
            for (j, v) in data[offset..offset + self.hidden_size].iter().enumerate() {
                pooled[j] += v;
            }
            count += 1;
        }
        if count > 0 {
            let n = count as f32;
            pooled.iter_mut().for_each(|v| *v /= n);
        }
        let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 {
            pooled.iter_mut().for_each(|v| *v /= norm);
        }
        Ok(pooled)
    }
}
