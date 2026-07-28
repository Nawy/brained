use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::domain::Domain;
use crate::hashing::hash_file;
use crate::scan::{index_text_content, SharedState};
use crate::store::{upsert_file_record, FileRecord};
use crate::types::FileKind;

/// Rejects any relative path that could escape `<domain>/work/` (`..`, absolute paths) or
/// that names a `knowledge` component — structural containment rather than a blocklist
/// against a separate `knowledge`-writing tool. Pure/unit-testable in isolation.
pub fn validate_work_relative_path(rel_path: &str) -> anyhow::Result<()> {
    let path = Path::new(rel_path);
    anyhow::ensure!(!path.is_absolute(), "path must be relative to <domain>/work/, got absolute path {rel_path:?}");
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                anyhow::ensure!(part != "knowledge", "path {rel_path:?} may not contain a \"knowledge\" component");
            }
            std::path::Component::CurDir => {}
            other => anyhow::bail!("path {rel_path:?} contains an disallowed component {other:?} (no `..`, no absolute paths)"),
        }
    }
    Ok(())
}

fn is_locked_by_us(state: &SharedState, domain: Domain) -> bool {
    let flag = match domain {
        Domain::Tech => &state.tech_locked_by_us,
        Domain::Business => &state.business_locked_by_us,
    };
    flag.load(std::sync::atomic::Ordering::SeqCst)
}

/// Writes `content` to `<domain>/work/<rel_path>` under `state.root`, then re-indexes it —
/// except for `(Tech, Work)`, which is disk-write-only (tech/work is never scanned/embedded,
/// see `walk.rs`). Requires the caller to currently hold `domain`'s lock (checked via the
/// in-memory `*_locked_by_us` flags on `SharedState`), even if the domain isn't held by
/// anyone else — the lock must be actively acquired first.
pub async fn write_work_file(state: &SharedState, domain: Domain, rel_path: &str, content: &str) -> anyhow::Result<()> {
    validate_work_relative_path(rel_path)?;
    anyhow::ensure!(
        is_locked_by_us(state, domain),
        "{} is not locked by this session — call lock{}() first",
        domain.as_str(),
        domain.as_str()
    );

    let work_subdir = format!("{}/work", domain.as_str());
    let target: PathBuf = state.root.join(&work_subdir).join(rel_path);

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&target, content).with_context(|| format!("writing {}", target.display()))?;

    let full_rel_path = format!("{work_subdir}/{rel_path}");

    if matches!(domain, Domain::Tech) {
        // tech/work is writable but never scanned/embedded/searchable — disk write only.
        return Ok(());
    }

    index_text_content(state, &full_rel_path, domain, FileKind::Work, content).await?;

    let source_hash = hash_file(&target)?;
    upsert_file_record(
        &*state.conn.lock().await,
        &FileRecord {
            path: full_rel_path,
            xxh3: source_hash,
            mtime: target.metadata()?.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
            domain,
            file_type: FileKind::Work,
            source_kind: "md".to_string(),
            last_scanned: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_relative_paths() {
        assert!(validate_work_relative_path("notes.md").is_ok());
        assert!(validate_work_relative_path("debug-notes/2026-07-27.md").is_ok());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(validate_work_relative_path("../knowledge/secret.md").is_err());
        assert!(validate_work_relative_path("a/../../b.md").is_err());
    }

    #[test]
    fn rejects_absolute_paths() {
        assert!(validate_work_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_any_knowledge_component() {
        assert!(validate_work_relative_path("knowledge/file.md").is_err());
        assert!(validate_work_relative_path("nested/knowledge/file.md").is_err());
    }
}
