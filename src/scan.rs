use std::path::PathBuf;

use ignore::gitignore::Gitignore;

use crate::config::Config;
use crate::convert::{convert_if_needed, Converter};
use crate::domain::Domain;
use crate::embed::Embedder;
use crate::hashing::hash_file;
use crate::store::{delete_file_record, get_file_record, list_all_paths, upsert_file_record, FileRecord};
use crate::types::FileKind;
use crate::vector_store::VectorStore;
use crate::walk::walk_folders;

pub struct SharedState {
    pub root: PathBuf,
    pub config: Config,
    pub spec: Gitignore,
    // Task 14 finding: rusqlite::Connection is Send but NOT Sync (its statement cache and inner
    // connection use RefCell internally) — confirmed by the compiler when BrdMcpServer's
    // `#[tool_router(server_handler)]`-derived `ServerHandler` impl (which requires
    // `Send + Sync + 'static`) failed to compile with `Arc<tokio::sync::RwLock<SharedState>>` as
    // a field, because `RwLock<T>: Sync` itself requires `T: Send + Sync`. Wrapping `conn` in its
    // own `tokio::sync::Mutex`, mirroring the existing `embedder` field, makes `SharedState: Sync`
    // again without touching any call site's borrow shape (`&*state.conn.lock().await` still
    // derefs to `&rusqlite::Connection`, so `store.rs`'s functions are unchanged). scan_once/
    // scan_one_file already hold `&mut SharedState` (exclusive), so the lock is uncontended there;
    // it only starts doing real work once BrdMcpServer's tool methods (Task 14) and the background
    // rescanning task (Task 16) can run concurrently against the same state.
    pub conn: tokio::sync::Mutex<rusqlite::Connection>,
    pub vector_store: VectorStore,
    pub embedder: tokio::sync::Mutex<Box<dyn Embedder + Send + Sync>>,
    pub converter: Box<dyn Converter + Send + Sync>,
    pub tech_locked_by_us: std::sync::atomic::AtomicBool,
    pub business_locked_by_us: std::sync::atomic::AtomicBool,
}

/// Chunks, embeds, and (re)stores `text` as the vector rows for `rel_path`. Shared by
/// `scan_one_file` (the normal scan path) and `write_work_file` (which already has the
/// content in hand as plain text and skips straight to this step, with no conversion or
/// hash-based skip involved).
pub async fn index_text_content(
    state: &SharedState,
    rel_path: &str,
    domain: Domain,
    kind: FileKind,
    text: &str,
) -> anyhow::Result<()> {
    let mut embedder = state.embedder.lock().await;
    let chunks = crate::chunk::chunk_text(text, state.config.chunk_size, embedder.tokenizer())?;
    let vectors = if chunks.is_empty() { Vec::new() } else { embedder.embed_documents(&chunks)? };
    drop(embedder);

    state.vector_store.delete_chunks_for_path(rel_path).await?;
    if !chunks.is_empty() {
        state.vector_store.add_chunks(rel_path, domain, kind, &chunks, &vectors).await?;
    }
    Ok(())
}

pub async fn scan_once(state: &mut SharedState) -> anyhow::Result<()> {
    let discovered = walk_folders(&state.root, &state.spec)?;
    let discovered_paths: std::collections::HashSet<String> =
        discovered.iter().map(|d| d.rel_path.clone()).collect();

    let stale: Vec<String> = {
        let conn = state.conn.lock().await;
        list_all_paths(&conn)?.difference(&discovered_paths).cloned().collect()
    };
    for path in stale {
        state.vector_store.delete_chunks_for_path(&path).await?;
        delete_file_record(&*state.conn.lock().await, &path)?;
    }

    // Bug fix (manual smoke test): scanning gave zero feedback for potentially many seconds
    // (model loading, then per-file hashing/chunking/embedding), which read as "frozen." All
    // output here goes to STDERR, never stdout: `scan_once` is shared by both `cmd_scan` (where
    // stdout doesn't matter) and `cmd_mcp` (where stdout IS the MCP JSON-RPC transport — writing
    // plain text there would corrupt the protocol stream).
    let total = discovered.len();
    eprintln!("brd: scanning {total} file{}...", if total == 1 { "" } else { "s" });
    let mut indexed = 0usize;
    for file in discovered {
        match scan_one_file(state, &file).await {
            Ok(true) => indexed += 1,
            Ok(false) => {}
            Err(err) => eprintln!("brd: skipping {}: {err}", file.rel_path),
        }
    }
    eprintln!("brd: scan complete — {indexed}/{total} file{} indexed or updated", if total == 1 { "" } else { "s" });
    Ok(())
}

/// Returns `Ok(true)` if the file was (re)indexed, `Ok(false)` if it was already up to date.
async fn scan_one_file(state: &mut SharedState, file: &crate::walk::DiscoveredFile) -> anyhow::Result<bool> {
    let source_hash = hash_file(&file.path)?;
    let previous = get_file_record(&*state.conn.lock().await, &file.rel_path)?;
    if let Some(record) = &previous {
        if record.xxh3 == source_hash {
            return Ok(false);
        }
    }
    eprintln!("brd:   {}", file.rel_path);

    let text = if matches!(file.source_kind.as_str(), "pdf" | "docx" | "pptx" | "xlsx" | "mp3" | "m4a") {
        let previous_hash = previous.as_ref().map(|r| r.xxh3.as_str());
        let target = convert_if_needed(&file.path, &source_hash, previous_hash, state.converter.as_ref())?;
        std::fs::read_to_string(&target)?
    } else {
        std::fs::read_to_string(&file.path)?
    };

    index_text_content(state, &file.rel_path, file.domain, file.file_type, &text).await?;

    upsert_file_record(
        &*state.conn.lock().await,
        &FileRecord {
            path: file.rel_path.clone(),
            xxh3: source_hash,
            mtime: file.path.metadata()?.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
            domain: file.domain,
            file_type: file.file_type,
            source_kind: file.source_kind.clone(),
            last_scanned: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
        },
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::convert::Converter;
    use crate::ignore_spec::build_ignore_spec;
    use crate::store::open_db;
    use crate::tokenizer::ChunkTokenizer;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    struct NoopConverter;
    impl Converter for NoopConverter {
        fn convert(&self, _path: &Path) -> anyhow::Result<String> {
            unreachable!("test fixtures only use .md/.txt files, which never need conversion")
        }
    }

    struct StubConverter {
        text: String,
    }
    impl Converter for StubConverter {
        fn convert(&self, _path: &Path) -> anyhow::Result<String> {
            Ok(self.text.clone())
        }
    }

    #[derive(Clone)]
    struct WordTokenizer;
    impl ChunkTokenizer for WordTokenizer {
        fn encode(&self, text: &str) -> Vec<u32> {
            text.split_whitespace().enumerate().map(|(i, _)| i as u32).collect()
        }
        fn decode(&self, ids: &[u32]) -> String {
            "x ".repeat(ids.len()).trim().to_string()
        }
    }

    async fn build_state(root: &Path, config: Config) -> SharedState {
        build_state_with_converter(root, config, Box::new(NoopConverter)).await
    }

    async fn build_state_with_converter(
        root: &Path,
        config: Config,
        converter: Box<dyn Converter + Send + Sync>,
    ) -> SharedState {
        let spec = build_ignore_spec(root, &config).unwrap();
        let conn = open_db(&root.join(".brained/state.db")).unwrap();
        let vector_store = crate::vector_store::VectorStore::open(&root.join(".brained/lancedb"), 4).await.unwrap();
        SharedState {
            root: root.to_path_buf(),
            config,
            spec,
            conn: tokio::sync::Mutex::new(conn),
            vector_store,
            embedder: tokio::sync::Mutex::new(Box::new(crate::embed::FakeEmbedder { tokenizer: WordTokenizer, dim: 4 })),
            converter,
            tech_locked_by_us: AtomicBool::new(false),
            business_locked_by_us: AtomicBool::new(false),
        }
    }

    #[tokio::test]
    async fn scan_indexes_new_files_and_removes_stale_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/a.md"), "hello world").unwrap();
        let mut state = build_state(dir.path(), Config::default()).await;

        scan_once(&mut state).await.unwrap();
        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "business/knowledge/a.md");

        std::fs::remove_file(dir.path().join("business/knowledge/a.md")).unwrap();
        scan_once(&mut state).await.unwrap();
        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert!(results.is_empty(), "chunks for a deleted file should be removed");
    }

    #[tokio::test]
    async fn unchanged_file_is_not_rescanned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/a.md"), "hello world").unwrap();
        let mut state = build_state(dir.path(), Config::default()).await;

        scan_once(&mut state).await.unwrap();
        let before = crate::store::get_file_record(&*state.conn.lock().await, "business/knowledge/a.md").unwrap().unwrap();
        scan_once(&mut state).await.unwrap();
        let after = crate::store::get_file_record(&*state.conn.lock().await, "business/knowledge/a.md").unwrap().unwrap();
        assert_eq!(before.xxh3, after.xxh3);
    }

    #[tokio::test]
    async fn unreadable_file_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/good.md"), "hello world").unwrap();
        std::fs::write(dir.path().join("business/knowledge/bad.md"), [0xFF, 0xFE, 0xFD]).unwrap(); // invalid UTF-8 — read_to_string will genuinely fail
        let mut state = build_state(dir.path(), Config::default()).await;

        let result = scan_once(&mut state).await;
        assert!(result.is_ok(), "one bad file must not fail the whole scan");
        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "business/knowledge/good.md");
    }

    #[tokio::test]
    async fn audio_files_are_converted_and_indexed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/memo.mp3"), "fake mp3 bytes").unwrap();
        let mut state = build_state_with_converter(
            dir.path(),
            Config::default(),
            Box::new(StubConverter { text: "hello from whisper".to_string() }),
        )
        .await;

        scan_once(&mut state).await.unwrap();

        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "business/knowledge/memo.mp3");
        assert_eq!(results[0].text, "hello from whisper");
    }

    #[tokio::test]
    async fn business_work_file_is_indexed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/work")).unwrap();
        std::fs::write(dir.path().join("business/work/notes.md"), "some working notes").unwrap();
        let mut state = build_state(dir.path(), Config::default()).await;

        scan_once(&mut state).await.unwrap();

        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "business/work/notes.md");
        assert!(matches!(results[0].domain, Domain::Business));
        assert!(matches!(results[0].file_type, FileKind::Work));
    }

    #[tokio::test]
    async fn tech_work_file_on_disk_is_never_indexed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tech/work")).unwrap();
        std::fs::write(dir.path().join("tech/work/debug-notes.md"), "scratch debugging notes").unwrap();
        let mut state = build_state(dir.path(), Config::default()).await;

        scan_once(&mut state).await.unwrap();

        let results = state.vector_store.query_chunks(&[0.0; 4], None, None, 10).await.unwrap();
        assert!(results.is_empty(), "tech/work must never be scanned/indexed, even though it's on disk");
        assert!(crate::store::get_file_record(&*state.conn.lock().await, "tech/work/debug-notes.md").unwrap().is_none());
    }
}
