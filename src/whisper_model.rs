use std::path::{Path, PathBuf};

use anyhow::Context;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
const MODEL_FILENAME: &str = "ggml-base.en.bin";

/// Downloads (or reuses a cached copy of) the English-only Whisper `base.en` model
/// into `<cache_dir>/ggml-base.en.bin`, returning its path. Not unit-tested directly
/// (needs network on first run) — mirrors `ort_runtime::ensure_onnxruntime_dylib`.
pub fn ensure_whisper_model(cache_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(cache_dir).with_context(|| format!("creating {}", cache_dir.display()))?;
    let model_path = cache_dir.join(MODEL_FILENAME);
    if model_path.exists() {
        return Ok(model_path);
    }

    eprintln!("brd: downloading Whisper base.en model (one-time, ~148MB)...");
    let response = ureq::get(MODEL_URL).call().with_context(|| format!("downloading {MODEL_URL}"))?;
    let partial_path = model_path.with_extension("bin.part");
    let mut partial_file =
        std::fs::File::create(&partial_path).with_context(|| format!("creating {}", partial_path.display()))?;
    std::io::copy(&mut response.into_body().into_reader(), &mut partial_file)
        .context("writing Whisper model response body")?;
    drop(partial_file);
    std::fs::rename(&partial_path, &model_path)
        .with_context(|| format!("renaming {} to {}", partial_path.display(), model_path.display()))?;
    eprintln!("brd: Whisper model ready");

    Ok(model_path)
}

/// Loads a `WhisperContext` from a model file on disk, CPU-only.
pub fn load_whisper_context(model_path: &Path) -> anyhow::Result<WhisperContext> {
    WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .with_context(|| format!("loading Whisper model from {}", model_path.display()))
}

/// Runs English transcription over 16kHz mono `f32` PCM samples, returning plain
/// text with no timestamps. Not unit-tested directly (needs a real loaded model) —
/// verified via the manual smoke test.
pub fn transcribe(ctx: &WhisperContext, samples: &[f32]) -> anyhow::Result<String> {
    let mut state = ctx.create_state().context("creating a Whisper inference state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_no_timestamps(true);
    // whisper.cpp prints its own diagnostics via C-level stdout/stderr writes; brd's
    // `mcp` command uses real stdout as the live MCP JSON-RPC transport, so nothing
    // from here may leak there.
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);

    state.full(params, samples).context("running Whisper inference")?;

    let mut text = String::new();
    for segment in state.as_iter() {
        let segment_text = segment.to_str_lossy().context("reading a Whisper segment's text")?;
        let trimmed = segment_text.trim();
        if !trimmed.is_empty() {
            text.push_str(trimmed);
            text.push('\n');
        }
    }
    Ok(text.trim().to_string())
}
