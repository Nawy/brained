use std::path::Path;

use anyhow::Context;

pub trait ChunkTokenizer {
    fn encode(&self, text: &str) -> Vec<u32>;
    fn decode(&self, ids: &[u32]) -> String;
}

pub struct HfTokenizer(pub tokenizers::Tokenizer);

impl ChunkTokenizer for HfTokenizer {
    fn encode(&self, text: &str) -> Vec<u32> {
        self.0
            .encode(text, false)
            .expect("tokenizer encode should not fail on well-formed UTF-8 input")
            .get_ids()
            .to_vec()
    }

    fn decode(&self, ids: &[u32]) -> String {
        self.0
            .decode(ids, true)
            .expect("tokenizer decode should not fail on ids it produced itself")
    }
}

/// Downloads (or reuses a cached copy of) nomic-embed-text-v1.5's tokenizer.json into
/// `cache_dir`, then loads it. Not unit-tested directly (needs network on first run) —
/// covered by the manual end-to-end smoke test (Task 17).
///
/// Deviates from the brief's sketch (`hf_hub::api::sync::Api::new()?.model(repo).get(filename)`):
/// the `hf-hub` version resolved by `cargo add` (1.0.0) replaced that API with a client/builder
/// shape (`HFClientSync::new()?.model(owner, name).download_file().filename(..).send()?`,
/// gated behind the `blocking` feature). Behavior — download-or-reuse-cached, then load — is
/// unchanged; only the call shape adapted to the real crate API (verified via the crate's
/// source under `~/.cargo/registry`, since `cargo doc` was unavailable in this sandbox).
pub fn fetch_tokenizer(cache_dir: &Path) -> anyhow::Result<HfTokenizer> {
    let dest = cache_dir.join("nomic-embed-text-v1.5-tokenizer.json");
    if !dest.exists() {
        std::fs::create_dir_all(cache_dir).with_context(|| format!("creating {}", cache_dir.display()))?;
        let client = hf_hub::HFClientSync::new().context("initializing hf-hub client")?;
        let repo = client.model("nomic-ai", "nomic-embed-text-v1.5");
        let downloaded = repo
            .download_file()
            .filename("tokenizer.json")
            .send()
            .context("downloading tokenizer.json")?;
        std::fs::copy(&downloaded, &dest)
            .with_context(|| format!("copying tokenizer into cache at {}", dest.display()))?;
    }
    let tokenizer = tokenizers::Tokenizer::from_file(&dest)
        .map_err(|e| anyhow::anyhow!("loading tokenizer from {}: {e}", dest.display()))?;
    Ok(HfTokenizer(tokenizer))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal real tokenizer.json (a WordLevel model over a tiny vocab) built in-memory,
    // so this test never touches the network or the real nomic-embed-text-v1.5 tokenizer.
    fn tiny_tokenizer() -> tokenizers::Tokenizer {
        use ahash::AHashMap;
        use tokenizers::models::wordlevel::WordLevel;
        use tokenizers::pre_tokenizers::whitespace::Whitespace;

        let mut vocab = AHashMap::new();
        for (i, word) in ["[UNK]", "hello", "world", "foo", "bar"].iter().enumerate() {
            vocab.insert(word.to_string(), i as u32);
        }
        let model = WordLevel::builder()
            .vocab(vocab)
            .unk_token("[UNK]".to_string())
            .build()
            .unwrap();
        let mut tok = tokenizers::Tokenizer::new(model);
        tok.with_pre_tokenizer(Some(Whitespace {}));
        tok
    }

    #[test]
    fn encode_decode_round_trips_known_words() {
        let hf = HfTokenizer(tiny_tokenizer());
        let ids = hf.encode("hello world");
        assert_eq!(ids.len(), 2);
        assert_eq!(hf.decode(&ids), "hello world");
    }

    #[test]
    fn unknown_words_map_to_unk() {
        let hf = HfTokenizer(tiny_tokenizer());
        let ids = hf.encode("hello nonexistentword");
        assert_eq!(ids.len(), 2);
        assert_ne!(hf.decode(&ids), "hello nonexistentword"); // second token is [UNK]
    }
}
