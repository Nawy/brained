use std::path::{Path, PathBuf};

use ignore::gitignore::Gitignore;

use crate::config::Config;
use crate::ignore_spec::is_ignored;
use crate::types::FileKind;

pub const SOURCE_EXTENSIONS: &[&str] = &["md", "txt", "pdf", "docx", "pptx", "xlsx", "mp3", "m4a"];

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub file_type: FileKind,
    pub source_kind: String,
}

fn is_knowledge_path(rel_path: &Path, knowledge_folders: &[PathBuf]) -> bool {
    knowledge_folders
        .iter()
        .any(|folder| rel_path == folder || rel_path.ancestors().any(|a| a == folder))
}

pub fn walk_folders(root: &Path, config: &Config, spec: &Gitignore) -> anyhow::Result<Vec<DiscoveredFile>> {
    let knowledge_folders: Vec<PathBuf> = config.knowledge_folders.iter().map(PathBuf::from).collect();
    let mut results = Vec::new();
    walk_dir(root, root, &knowledge_folders, spec, &mut results)?;
    Ok(results)
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    knowledge_folders: &[PathBuf],
    spec: &Gitignore,
    results: &mut Vec<DiscoveredFile>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if is_ignored(spec, &rel, true) {
                continue;
            }
            walk_dir(root, &path, knowledge_folders, spec, results)?;
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
            let kind = if is_knowledge_path(&rel, knowledge_folders) {
                FileKind::Knowledge
            } else {
                FileKind::Research
            };
            results.push(DiscoveredFile {
                path: path.clone(),
                rel_path: rel.to_string_lossy().replace('\\', "/"),
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
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::create_dir_all(dir.path().join("research")).unwrap();
        std::fs::write(dir.path().join("knowledge/guide.md"), "hi").unwrap();
        std::fs::write(dir.path().join("research/todo.txt"), "hi").unwrap();
        std::fs::write(dir.path().join("ignored.png"), "hi").unwrap();
        dir
    }

    #[test]
    fn classifies_knowledge_vs_research() {
        let dir = setup();
        let config = Config { knowledge_folders: vec!["knowledge".to_string()], ..Config::default() };
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &config, &spec).unwrap();

        let guide = found.iter().find(|f| f.rel_path == "knowledge/guide.md").unwrap();
        assert!(matches!(guide.file_type, crate::types::FileKind::Knowledge));
        assert_eq!(guide.source_kind, "md");

        let todo = found.iter().find(|f| f.rel_path == "research/todo.txt").unwrap();
        assert!(matches!(todo.file_type, crate::types::FileKind::Research));
    }

    #[test]
    fn discovers_and_classifies_audio_extensions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("knowledge")).unwrap();
        std::fs::write(dir.path().join("knowledge/memo.mp3"), "fake mp3 bytes").unwrap();
        std::fs::write(dir.path().join("interview.m4a"), "fake m4a bytes").unwrap();
        let config = Config { knowledge_folders: vec!["knowledge".to_string()], ..Config::default() };
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &config, &spec).unwrap();

        let memo = found.iter().find(|f| f.rel_path == "knowledge/memo.mp3").unwrap();
        assert!(matches!(memo.file_type, crate::types::FileKind::Knowledge));
        assert_eq!(memo.source_kind, "mp3");

        let interview = found.iter().find(|f| f.rel_path == "interview.m4a").unwrap();
        assert!(matches!(interview.file_type, crate::types::FileKind::Research));
        assert_eq!(interview.source_kind, "m4a");
    }

    #[test]
    fn skips_unsupported_extensions() {
        let dir = setup();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &config, &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path == "ignored.png"));
    }

    #[test]
    fn hardcoded_ignores_prune_directories() {
        let dir = setup();
        std::fs::create_dir_all(dir.path().join(".brained")).unwrap();
        std::fs::write(dir.path().join(".brained/state.db"), "hi").unwrap();
        std::fs::write(dir.path().join(".brained/note.md"), "hi").unwrap();
        let config = Config::default();
        let spec = build_ignore_spec(dir.path(), &config).unwrap();
        let found = walk_folders(dir.path(), &config, &spec).unwrap();
        assert!(!found.iter().any(|f| f.rel_path.starts_with(".brained")));
    }
}
