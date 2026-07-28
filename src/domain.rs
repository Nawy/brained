use std::path::Path;

use crate::types::FileKind;

pub const BUSINESS_KNOWLEDGE: &str = "business/knowledge";
pub const BUSINESS_WORK: &str = "business/work";
pub const TECH_KNOWLEDGE: &str = "tech/knowledge";
pub const TECH_WORK: &str = "tech/work";

/// The four mandatory folders created by `brd init`, in the order `brd init` creates them.
pub const ALL_FOLDERS: &[&str] = &[BUSINESS_KNOWLEDGE, BUSINESS_WORK, TECH_KNOWLEDGE, TECH_WORK];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Business,
    Tech,
}

impl Domain {
    pub fn as_str(&self) -> &'static str {
        match self {
            Domain::Business => "business",
            Domain::Tech => "tech",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Domain> {
        match s {
            "business" => Ok(Domain::Business),
            "tech" => Ok(Domain::Tech),
            other => anyhow::bail!("unknown domain {other:?}, expected \"business\" or \"tech\""),
        }
    }
}

fn is_under(rel_path: &Path, folder: &str) -> bool {
    rel_path.ancestors().any(|a| a == Path::new(folder))
}

/// Classifies a project-root-relative path into `(Domain, FileKind)` based on which of the
/// four mandatory folders it falls under. Returns `None` if the path is outside all four —
/// such a file is not part of the indexed tree at all (not scanned, not writable via
/// `write_work_file`).
pub fn classify_path(rel_path: &Path) -> Option<(Domain, FileKind)> {
    if is_under(rel_path, BUSINESS_KNOWLEDGE) {
        Some((Domain::Business, FileKind::Knowledge))
    } else if is_under(rel_path, BUSINESS_WORK) {
        Some((Domain::Business, FileKind::Work))
    } else if is_under(rel_path, TECH_KNOWLEDGE) {
        Some((Domain::Tech, FileKind::Knowledge))
    } else if is_under(rel_path, TECH_WORK) {
        Some((Domain::Tech, FileKind::Work))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn round_trips_through_str() {
        assert_eq!(Domain::parse("business").unwrap(), Domain::Business);
        assert_eq!(Domain::parse("tech").unwrap(), Domain::Tech);
        assert_eq!(Domain::Business.as_str(), "business");
        assert_eq!(Domain::Tech.as_str(), "tech");
        assert!(Domain::parse("bogus").is_err());
    }

    #[test]
    fn classifies_all_four_folders() {
        assert_eq!(
            classify_path(&PathBuf::from("business/knowledge/guide.md")),
            Some((Domain::Business, FileKind::Knowledge))
        );
        assert_eq!(
            classify_path(&PathBuf::from("business/work/notes.md")),
            Some((Domain::Business, FileKind::Work))
        );
        assert_eq!(
            classify_path(&PathBuf::from("tech/knowledge/incident-123.md")),
            Some((Domain::Tech, FileKind::Knowledge))
        );
        assert_eq!(
            classify_path(&PathBuf::from("tech/work/debug-notes.md")),
            Some((Domain::Tech, FileKind::Work))
        );
    }

    #[test]
    fn classifies_nested_paths_under_a_folder() {
        assert_eq!(
            classify_path(&PathBuf::from("tech/knowledge/2026/incident.md")),
            Some((Domain::Tech, FileKind::Knowledge))
        );
    }

    #[test]
    fn path_outside_all_four_folders_is_none() {
        assert_eq!(classify_path(&PathBuf::from("README.md")), None);
        assert_eq!(classify_path(&PathBuf::from("business/other/file.md")), None);
        assert_eq!(classify_path(&PathBuf::from("random/nested/file.md")), None);
    }
}
