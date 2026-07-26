use crate::tokenizer::ChunkTokenizer;

pub fn chunk_text(text: &str, chunk_size: u32, tokenizer: &dyn ChunkTokenizer) -> anyhow::Result<Vec<String>> {
    if chunk_size == 0 {
        anyhow::bail!("chunk_size must be positive");
    }
    let chunk_size = chunk_size as usize;

    let paragraphs: Vec<&str> = text.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    let mut chunks = Vec::new();
    let mut current_paragraphs: Vec<&str> = Vec::new();
    let mut current_tokens: usize = 0;

    for para in paragraphs {
        let para_tokens = tokenizer.encode(para);
        let para_len = para_tokens.len();

        if para_len > chunk_size {
            if !current_paragraphs.is_empty() {
                chunks.push(current_paragraphs.join("\n\n"));
                current_paragraphs.clear();
                current_tokens = 0;
            }
            for start in (0..para_len).step_by(chunk_size) {
                let end = (start + chunk_size).min(para_len);
                chunks.push(tokenizer.decode(&para_tokens[start..end]));
            }
            continue;
        }

        if !current_paragraphs.is_empty() && current_tokens + para_len > chunk_size {
            chunks.push(current_paragraphs.join("\n\n"));
            current_paragraphs.clear();
            current_tokens = 0;
        }

        current_paragraphs.push(para);
        current_tokens += para_len;
    }

    if !current_paragraphs.is_empty() {
        chunks.push(current_paragraphs.join("\n\n"));
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::ChunkTokenizer;

    /// One token per whitespace-separated word — makes chunk-size math easy to predict in tests.
    struct WordTokenizer;
    impl ChunkTokenizer for WordTokenizer {
        fn encode(&self, text: &str) -> Vec<u32> {
            text.split_whitespace().enumerate().map(|(i, _)| i as u32).collect()
        }
        fn decode(&self, ids: &[u32]) -> String {
            // Not exact round-trip decoding (we don't keep the original words per id here) —
            // only `ids.len()` (the token count) matters for these chunk-boundary tests.
            "x ".repeat(ids.len()).trim().to_string()
        }
    }

    #[test]
    fn packs_multiple_short_paragraphs_into_one_chunk() {
        let text = "one two\n\nthree four";
        let chunks = chunk_text(text, 10, &WordTokenizer).unwrap();
        assert_eq!(chunks, vec!["one two\n\nthree four".to_string()]);
    }

    #[test]
    fn splits_when_next_paragraph_would_exceed_chunk_size() {
        let text = "one two three\n\nfour five six";
        let chunks = chunk_text(text, 4, &WordTokenizer).unwrap();
        assert_eq!(chunks, vec!["one two three".to_string(), "four five six".to_string()]);
    }

    #[test]
    fn oversized_single_paragraph_is_split_by_tokens() {
        let text = "one two three four five six";
        let chunks = chunk_text(text, 2, &WordTokenizer).unwrap();
        assert_eq!(chunks, vec!["x x".to_string(), "x x".to_string(), "x x".to_string()]);
    }

    #[test]
    fn empty_text_produces_no_chunks() {
        assert_eq!(chunk_text("", 800, &WordTokenizer).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn zero_chunk_size_is_an_error() {
        assert!(chunk_text("hello", 0, &WordTokenizer).is_err());
    }
}
