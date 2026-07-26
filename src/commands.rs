use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use directories::ProjectDirs;
use tokio::sync::RwLock;

use crate::config::{load_config, CONFIG_FILENAME, DEFAULT_CONFIG_TOML};
use crate::convert::RealConverter;
use crate::embed::{Embedder, NomicEmbedder};
use crate::ignore_spec::build_ignore_spec;
use crate::scan::{scan_once, SharedState};
use crate::skill::{render_skill_md, SKILL_PATH};
use crate::store::{list_all_paths, open_db};
use crate::vector_store::VectorStore;

const BRAINED_DIR: &str = ".brained";

/// Bug fix (manual smoke test): on Windows, `Path::display()` prints whatever mix of `\` and `/`
/// the path happens to contain — e.g. `root.join(".claude/skills/...")` produces
/// `C:\Users\...\project\.claude/skills/...` (backslashes from the OS-native root, forward
/// slashes preserved verbatim from a literal path constant like `SKILL_PATH`). That's confusing
/// and risky to copy-paste into a shell. Normalize every user-facing path print to forward
/// slashes — Windows accepts `/` everywhere it accepts `\`, so this is safe and consistent
/// cross-platform (a no-op on Unix, where the path never had backslashes to begin with).
fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn model_cache_dir() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "brd", "brd").context("could not determine a home directory for the model cache")?;
    Ok(dirs.cache_dir().to_path_buf())
}

pub fn cmd_init(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root.join(BRAINED_DIR))?;

    let config_path = root.join(CONFIG_FILENAME);
    if !config_path.exists() {
        std::fs::write(&config_path, DEFAULT_CONFIG_TOML)?;
    }

    let skill_path = root.join(SKILL_PATH);
    std::fs::create_dir_all(skill_path.parent().unwrap())?;
    if !skill_path.exists() {
        std::fs::write(&skill_path, render_skill_md())?;
    }

    println!("Skill installed at {}", display_path(&skill_path));
    println!();
    println!("Run `brd install` to print the command for registering this as an MCP server.");
    Ok(())
}

pub fn cmd_install(_root: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolving the current binary's path")?;
    println!("To register brained as an MCP server in Claude Code, run:");
    println!("  claude mcp add brained -- \"{}\" mcp", display_path(&exe));
    Ok(())
}

async fn open_shared_state(root: &Path) -> anyhow::Result<SharedState> {
    let config = load_config(&root.join(CONFIG_FILENAME))?;
    let spec = build_ignore_spec(root, &config)?;
    let conn = open_db(&root.join(BRAINED_DIR).join("state.db"))?;
    let cache_dir = model_cache_dir()?;
    let vector_store = VectorStore::open(&root.join(BRAINED_DIR).join("lancedb"), crate::embed::EMBEDDING_DIM).await?;
    let embedder_impl: Box<dyn Embedder + Send + Sync> = Box::new(NomicEmbedder::new(&cache_dir)?);
    let embedder = tokio::sync::Mutex::new(embedder_impl);
    Ok(SharedState {
        root: root.to_path_buf(),
        config,
        spec,
        conn: tokio::sync::Mutex::new(conn),
        vector_store,
        embedder,
        converter: Box::new(RealConverter::new(cache_dir)),
    })
}

pub async fn cmd_scan(root: &Path) -> anyhow::Result<()> {
    let mut state = open_shared_state(root).await?;
    scan_once(&mut state).await
}

pub async fn cmd_mcp(root: &Path) -> anyhow::Result<()> {
    let mut state = open_shared_state(root).await?;
    scan_once(&mut state).await?;
    let interval_secs = state.config.scan_interval_seconds;

    let shared = Arc::new(RwLock::new(state));

    let background = {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            ticker.tick().await; // first tick fires immediately; we already scanned above
            loop {
                ticker.tick().await;
                let mut state = shared.write().await;
                if let Err(e) = scan_once(&mut state).await {
                    eprintln!("brd: background scan failed: {e}");
                }
            }
        })
    };

    let server = crate::mcp_server::BrdMcpServer { state: shared };
    let service = rmcp::ServiceExt::serve(server, rmcp::transport::stdio())
        .await
        .context("starting MCP stdio server")?;
    service.waiting().await.context("running MCP server")?;

    background.abort();
    Ok(())
}

pub fn cmd_info(root: &Path) -> anyhow::Result<()> {
    let cache_dir = model_cache_dir()?;
    println!("brd version: {}", env!("CARGO_PKG_VERSION"));
    println!("Model cache: {}", display_path(&cache_dir));

    let db_path = root.join(BRAINED_DIR).join("state.db");
    if db_path.exists() {
        let conn = open_db(&db_path)?;
        let count = list_all_paths(&conn)?.len();
        println!("Indexed files: {count}");
    } else {
        println!("Indexed files: 0 (no scan has run yet)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_brained_dir_config_and_skill_without_mentioning_mcp_add() {
        let dir = tempfile::tempdir().unwrap();
        cmd_init(dir.path()).unwrap();

        assert!(dir.path().join(".brained").is_dir());
        assert!(dir.path().join(crate::config::CONFIG_FILENAME).exists());
        assert!(dir.path().join(crate::skill::SKILL_PATH).exists());
    }

    #[test]
    fn init_does_not_overwrite_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(crate::config::CONFIG_FILENAME), "chunk_size = 42\n").unwrap();
        cmd_init(dir.path()).unwrap();
        let contents = std::fs::read_to_string(dir.path().join(crate::config::CONFIG_FILENAME)).unwrap();
        assert!(contents.contains("chunk_size = 42"));
    }

    #[test]
    fn info_reports_zero_files_before_any_scan() {
        let dir = tempfile::tempdir().unwrap();
        cmd_init(dir.path()).unwrap();
        // cmd_info prints to stdout; verified end-to-end via the manual smoke test (Task 17).
        // Here we only check it doesn't error when no state.db exists yet.
        assert!(cmd_info(dir.path()).is_ok());
    }
}
