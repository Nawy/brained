use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

pub const DEFAULT_CHUNK_SIZE: u32 = 800;
pub const DEFAULT_SCAN_INTERVAL: u64 = 30;
pub const CONFIG_FILENAME: &str = ".brained.config.toml";

pub const DEFAULT_CONFIG_TOML: &str = "knowledge_folders = []\nchunk_size = 800\nscan_interval_seconds = 30\nignore = []\n";

#[derive(Debug, Clone, Deserialize)]
struct RawConfig {
    #[serde(default)]
    knowledge_folders: Vec<String>,
    #[serde(default)]
    chunk_size: Option<u32>,
    #[serde(default)]
    scan_interval_seconds: Option<u64>,
    #[serde(default)]
    ignore: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub knowledge_folders: Vec<String>,
    pub chunk_size: u32,
    pub scan_interval_seconds: u64,
    pub ignore: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            knowledge_folders: Vec::new(),
            chunk_size: DEFAULT_CHUNK_SIZE,
            scan_interval_seconds: DEFAULT_SCAN_INTERVAL,
            ignore: Vec::new(),
        }
    }
}

pub fn load_config(config_path: &Path) -> anyhow::Result<Config> {
    if !config_path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let raw: RawConfig = toml::from_str(&text)
        .with_context(|| format!("parsing {} (check the `ignore` and `knowledge_folders` keys are lists)", config_path.display()))?;
    Ok(Config {
        knowledge_folders: raw.knowledge_folders,
        chunk_size: raw.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
        scan_interval_seconds: raw.scan_interval_seconds.unwrap_or(DEFAULT_SCAN_INTERVAL),
        ignore: raw.ignore,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".brained.config.toml");
        let config = load_config(&path).unwrap();
        assert_eq!(config.knowledge_folders, Vec::<String>::new());
        assert_eq!(config.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(config.scan_interval_seconds, DEFAULT_SCAN_INTERVAL);
        assert_eq!(config.ignore, Vec::<String>::new());
    }

    #[test]
    fn valid_toml_overrides_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".brained.config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"knowledge_folders = ["docs"]
chunk_size = 400
scan_interval_seconds = 10
ignore = ["*.tmp"]"#
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.knowledge_folders, vec!["docs".to_string()]);
        assert_eq!(config.chunk_size, 400);
        assert_eq!(config.scan_interval_seconds, 10);
        assert_eq!(config.ignore, vec!["*.tmp".to_string()]);
    }

    #[test]
    fn ignore_not_a_list_is_a_clean_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".brained.config.toml");
        std::fs::write(&path, r#"ignore = "not-a-list""#).unwrap();
        let err = load_config(&path).unwrap_err();
        assert!(err.to_string().contains("ignore"), "error was: {err}");
    }
}
