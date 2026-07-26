use std::path::Path;

use anyhow::Context;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::tokenizer::{fetch_tokenizer, HfTokenizer};

pub const EMBEDDING_DIM: i32 = 768;

fn with_document_prefix(text: &str) -> String {
    format!("search_document: {text}")
}

fn with_query_prefix(text: &str) -> String {
    format!("search_query: {text}")
}

/// Behavior behind `NomicEmbedder`'s real ONNX model, so `scan_once` and its tests can depend on
/// a trait object instead of the concrete type — `NomicEmbedder` wraps a real
/// `fastembed::TextEmbedding`, which needs a multi-hundred-MB model download on first use, and
/// unit tests use `FakeEmbedder` below instead.
pub trait Embedder {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
    fn embed_query(&mut self, text: &str) -> anyhow::Result<Vec<f32>>;
    fn tokenizer(&self) -> &dyn crate::tokenizer::ChunkTokenizer;
}

// Task 9 Step 2 finding: fastembed::TextEmbedding::embed takes `&mut self`, NOT `&self`
// (confirmed by reading fastembed-5.17.3/src/text_embedding/impl.rs — `pub fn embed<S: ...>(&mut self, ...)`).
// This deviates from the brief's assumption. Consequence for Task 14: NomicEmbedder cannot be
// called concurrently through a shared read guard; the embedder must be wrapped in its own
// `tokio::sync::Mutex<NomicEmbedder>` inside SharedState so calls into it serialize, without
// forcing the whole read path (which also touches the store) to serialize.
pub struct NomicEmbedder {
    model: TextEmbedding,
    pub tokenizer: HfTokenizer,
}

impl NomicEmbedder {
    pub fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(cache_dir).with_context(|| format!("creating {}", cache_dir.display()))?;

        // Bug fix: point `ort` at a known-compatible ONNX Runtime dylib before it ever falls
        // back to searching the OS's default dynamic-library path (which on Windows was
        // confirmed to silently resolve to an incompatible OS-bundled copy and hang — see the
        // Cargo.toml comment on the `ort` dependency and `ort_runtime.rs` for the full story;
        // Linux/macOS usually have nothing on the default path to find at all). Must happen
        // before the first `TextEmbedding`/`Session` is created anywhere in the process.
        // Windows path verified via live repro + fix; Linux (x86_64)/macOS (aarch64) implemented
        // from Microsoft's real published release assets but not run on that hardware — see
        // ort_runtime.rs's module doc for exactly what was and wasn't verified.
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        {
            let dylib_path = crate::ort_runtime::ensure_onnxruntime_dylib(cache_dir)?;
            let committed = ort::init_from(&dylib_path)
                .with_context(|| format!("loading ONNX Runtime from {}", dylib_path.display()))?
                .commit();
            if !committed {
                eprintln!(
                    "brd: warning: an ONNX Runtime environment was already configured before {} could be applied",
                    dylib_path.display()
                );
            }
        }

        // Bug fix: model download has its own progress bar (with_show_download_progress), but
        // ONNX Runtime session creation after that (and the tokenizer fetch below) were both
        // silent — the gap between "download finished" and "scan starts" read as a hang.
        eprintln!("brd: loading embedding model...");
        let model = TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::NomicEmbedTextV15)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(true),
        )
        .context("loading nomic-embed-text-v1.5 model")?;
        let tokenizer = fetch_tokenizer(cache_dir)?;
        eprintln!("brd: embedding model ready");
        Ok(NomicEmbedder { model, tokenizer })
    }

    pub fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts.iter().map(|t| with_document_prefix(t)).collect();
        self.model.embed(prefixed, None).context("embedding documents")
    }

    pub fn embed_query(&mut self, text: &str) -> anyhow::Result<Vec<f32>> {
        let prefixed = vec![with_query_prefix(text)];
        let mut result = self.model.embed(prefixed, None).context("embedding query")?;
        result.pop().context("embed() returned no vectors for a single query")
    }
}

impl Embedder for NomicEmbedder {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        NomicEmbedder::embed_documents(self, texts)
    }
    fn embed_query(&mut self, text: &str) -> anyhow::Result<Vec<f32>> {
        NomicEmbedder::embed_query(self, text)
    }
    fn tokenizer(&self) -> &dyn crate::tokenizer::ChunkTokenizer {
        &self.tokenizer
    }
}

/// Content-independent fake: fills in a zero vector of the right dimension for every input.
/// scan_once's tests only exercise orchestration (walk → hash → chunk → embed → store), never
/// embedding *quality*, so the vectors' values don't matter — only their shape does.
#[cfg(test)]
pub struct FakeEmbedder<T: crate::tokenizer::ChunkTokenizer> {
    pub tokenizer: T,
    pub dim: usize,
}

#[cfg(test)]
impl<T: crate::tokenizer::ChunkTokenizer> Embedder for FakeEmbedder<T> {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0f32; self.dim]).collect())
    }
    fn embed_query(&mut self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0f32; self.dim])
    }
    fn tokenizer(&self) -> &dyn crate::tokenizer::ChunkTokenizer {
        &self.tokenizer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_prefix_is_prepended() {
        assert_eq!(with_document_prefix("hello"), "search_document: hello");
    }

    #[test]
    fn query_prefix_is_prepended() {
        assert_eq!(with_query_prefix("hello"), "search_query: hello");
    }
}
