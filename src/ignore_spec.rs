use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

use crate::config::Config;

pub const HARDCODED_IGNORES: &[&str] = &[".git/", ".claude/", ".brained/", "*.b.md"];

pub fn build_ignore_spec(root: &Path, config: &Config) -> anyhow::Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in HARDCODED_IGNORES.iter().copied().chain(config.ignore.iter().map(String::as_str)) {
        builder.add_line(None, pattern)?;
    }
    Ok(builder.build()?)
}

pub fn is_ignored(spec: &Gitignore, rel_path: &Path, is_dir: bool) -> bool {
    matches!(
        spec.matched_path_or_any_parents(rel_path, is_dir),
        Match::Ignore(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    fn spec(extra_ignore: Vec<String>) -> ignore::gitignore::Gitignore {
        let config = Config { ignore: extra_ignore, ..Config::default() };
        build_ignore_spec(Path::new("/project"), &config).unwrap()
    }

    #[test]
    fn hardcoded_ignores_apply() {
        let spec = spec(vec![]);
        assert!(is_ignored(&spec, Path::new(".git"), true));
        assert!(is_ignored(&spec, Path::new(".brained"), true));
        assert!(is_ignored(&spec, Path::new("notes.b.md"), false));
        assert!(!is_ignored(&spec, Path::new("notes.md"), false));
    }

    #[test]
    fn config_ignore_patterns_apply() {
        let spec = spec(vec!["*.tmp".to_string(), "build/".to_string()]);
        assert!(is_ignored(&spec, Path::new("scratch.tmp"), false));
        assert!(is_ignored(&spec, Path::new("build"), true));
        assert!(!is_ignored(&spec, Path::new("build.rs"), false));
    }

    #[test]
    fn nested_file_under_ignored_dir_is_ignored() {
        let spec = spec(vec![]);
        assert!(is_ignored(&spec, Path::new(".git/HEAD"), false));
    }
}
