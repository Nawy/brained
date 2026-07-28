use std::path::{Path, PathBuf};

use ignore::gitignore::Gitignore;

use crate::domain::{classify_path, Domain};
use crate::ignore_spec::is_ignored;
use crate::types::FileKind;

pub const SOURCE_EXTENSIONS: &[&str] = &["md", "txt", "pdf", "docx", "pptx", "xlsx", "mp3", "m4a"];

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub domain: Domain,
    pub file_type: FileKind,
    pub source_kind: String,
}

pub fn walk_folders(root: &Path, spec: &Gitignore) -> anyhow::Result<Vec<DiscoveredFile>> {
    let mut results = Vec::new();
    walk_dir(root, root, spec, &mut results)?;
    Ok(results)
}

fn walk_dir(root: &Path, dir: &Path, spec: &Gitignore, results: &mut Vec<DiscoveredFile>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if is_ignored(spec, &rel, true) {
                continue;
            }
            walk_dir(root, &path, spec, results)?;
        } else if file_type.is_file() {
            if is_ignored(spec, &rel, false) {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) else {
                continue;
            };
            if !SOURCE_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            // Only files under one of the four mandatory business/tech knowledge/work
            // folders are part of the indexed tree at all; anything else is skipped, same
            // treatment as an unsupported extension.
            let Some((domain, kind)) = classify_path(&rel) else {
                continue;
            };
            // tech/work is writable (via write_work_file) but must never be
            // scanned/embedded/searchable — skip it here so it never even reaches
            // scan_one_file, which is the simplest way to guarantee that property.
            if matches!((domain, kind), (Domain::Tech, FileKind::Work)) {
                continue;
            }
            results.push(DiscoveredFile {
                path: path.clone(),
                rel_path: rel.to_string_lossy().replace('\\', "/"),
                domain,
                file_type: kind,
                source_kind: ext,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ignore_spec::build_ignore_spec;

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::create_dir_all(dir.path().join("business/work")).unwrap();
        std::fs::create_dir_all(dir.path().join("tech/knowledge")).unwrap();
        std::fs::create_dir_all(dir.path().join("tech/work")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/guide.md"), "hi").unwrap();
        std::fs::write(dir.path().join("business/work/todo.txt"), "hi").unwrap();
        std::fs::write(dir.path().join("tech/knowledge/incident.md"), "hi").unwrap();
        std::fs::write(dir.path().join("tech/work/debug-notes.md"), "hi").unwrap();
        std::fs::write(dir.path().join("ignored.png"), "hi").unwrap();
        dir
    }

    #[test]
    fn classifies_all_four_folders() {
        let dir = setup();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();

        let guide = found.iter().find(|f| f.rel_path == "business/knowledge/guide.md").unwrap();
        assert!(matches!(guide.domain, Domain::Business));
        assert!(matches!(guide.file_type, FileKind::Knowledge));
        assert_eq!(guide.source_kind, "md");

        let todo = found.iter().find(|f| f.rel_path == "business/work/todo.txt").unwrap();
        assert!(matches!(todo.domain, Domain::Business));
        assert!(matches!(todo.file_type, FileKind::Work));

        let incident = found.iter().find(|f| f.rel_path == "tech/knowledge/incident.md").unwrap();
        assert!(matches!(incident.domain, Domain::Tech));
        assert!(matches!(incident.file_type, FileKind::Knowledge));
    }

    #[test]
    fn tech_work_files_are_never_discovered() {
        let dir = setup();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path == "tech/work/debug-notes.md"));
    }

    #[test]
    fn discovers_and_classifies_audio_extensions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("business/knowledge")).unwrap();
        std::fs::write(dir.path().join("business/knowledge/memo.mp3"), "fake mp3 bytes").unwrap();
        std::fs::create_dir_all(dir.path().join("business/work")).unwrap();
        std::fs::write(dir.path().join("business/work/interview.m4a"), "fake m4a bytes").unwrap();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();

        let memo = found.iter().find(|f| f.rel_path == "business/knowledge/memo.mp3").unwrap();
        assert!(matches!(memo.file_type, FileKind::Knowledge));
        assert_eq!(memo.source_kind, "mp3");

        let interview = found.iter().find(|f| f.rel_path == "business/work/interview.m4a").unwrap();
        assert!(matches!(interview.file_type, FileKind::Work));
        assert_eq!(interview.source_kind, "m4a");
    }

    #[test]
    fn skips_unsupported_extensions() {
        let dir = setup();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path == "ignored.png"));
    }

    #[test]
    fn file_outside_all_four_folders_is_skipped() {
        let dir = setup();
        std::fs::write(dir.path().join("README.md"), "hi").unwrap();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path == "README.md"));
    }

    #[test]
    fn hardcoded_ignores_prune_directories() {
        let dir = setup();
        std::fs::create_dir_all(dir.path().join(".brained")).unwrap();
        std::fs::write(dir.path().join(".brained/state.db"), "hi").unwrap();
        std::fs::write(dir.path().join(".brained/note.md"), "hi").unwrap();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path.starts_with(".brained")));
    }
}
