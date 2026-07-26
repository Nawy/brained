use std::path::{Path, PathBuf};

use anyhow::Context;
use once_cell::sync::OnceCell;
use whisper_rs::WhisperContext;

pub fn b_md_path_for(source: &Path) -> PathBuf {
    let mut name = source.file_name().expect("source path must have a filename").to_os_string();
    name.push(".b.md");
    source.with_file_name(name)
}

pub trait Converter {
    fn convert(&self, path: &Path) -> anyhow::Result<String>;
}

pub struct RealConverter {
    model_cache_dir: PathBuf,
    whisper_ctx: OnceCell<WhisperContext>,
}

impl RealConverter {
    pub fn new(model_cache_dir: PathBuf) -> Self {
        RealConverter { model_cache_dir, whisper_ctx: OnceCell::new() }
    }
}

impl Converter for RealConverter {
    fn convert(&self, path: &Path) -> anyhow::Result<String> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        match ext.as_str() {
            "pdf" => pdf_extract::extract_text(path).with_context(|| format!("extracting text from {}", path.display())),
            "docx" | "pptx" | "xlsx" => {
                undoc::to_markdown(path).with_context(|| format!("converting {} to markdown", path.display()))
            }
            "mp3" | "m4a" => {
                let ctx = self.whisper_ctx.get_or_try_init(|| -> anyhow::Result<WhisperContext> {
                    let model_path = crate::whisper_model::ensure_whisper_model(&self.model_cache_dir)?;
                    eprintln!("brd: loading Whisper model...");
                    let ctx = crate::whisper_model::load_whisper_context(&model_path)?;
                    eprintln!("brd: Whisper model ready");
                    Ok(ctx)
                })?;
                let samples = crate::audio::decode_to_pcm16k_mono(path)?;
                crate::whisper_model::transcribe(ctx, &samples)
            }
            other => anyhow::bail!("no converter registered for .{other} files"),
        }
    }
}

pub fn convert_if_needed(
    source: &Path,
    source_hash: &str,
    previous_hash: Option<&str>,
    converter: &dyn Converter,
) -> anyhow::Result<PathBuf> {
    let target = b_md_path_for(source);
    if target.exists() && previous_hash == Some(source_hash) {
        return Ok(target);
    }
    let text = converter.convert(source)?;
    std::fs::write(&target, text).with_context(|| format!("writing {}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeConverter {
        calls: RefCell<Vec<std::path::PathBuf>>,
        text: String,
    }
    impl Converter for FakeConverter {
        fn convert(&self, path: &Path) -> anyhow::Result<String> {
            self.calls.borrow_mut().push(path.to_path_buf());
            Ok(self.text.clone())
        }
    }

    #[test]
    fn b_md_path_appends_suffix() {
        assert_eq!(
            b_md_path_for(Path::new("docs/report.pdf")),
            std::path::PathBuf::from("docs/report.pdf.b.md")
        );
    }

    #[test]
    fn converts_when_no_cached_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.pdf");
        std::fs::write(&source, "fake pdf bytes").unwrap();
        let converter = FakeConverter { calls: RefCell::new(vec![]), text: "# Report".to_string() };

        let target = convert_if_needed(&source, "hash1", None, &converter).unwrap();

        assert_eq!(converter.calls.borrow().len(), 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# Report");
    }

    #[test]
    fn skips_reconversion_when_hash_unchanged_and_sidecar_exists() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.pdf");
        std::fs::write(&source, "fake pdf bytes").unwrap();
        let converter = FakeConverter { calls: RefCell::new(vec![]), text: "# Report".to_string() };

        convert_if_needed(&source, "hash1", None, &converter).unwrap();
        convert_if_needed(&source, "hash1", Some("hash1"), &converter).unwrap();

        assert_eq!(converter.calls.borrow().len(), 1, "should not reconvert on unchanged hash");
    }

    #[test]
    fn reconverts_when_hash_changed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.pdf");
        std::fs::write(&source, "fake pdf bytes v2").unwrap();
        let converter = FakeConverter { calls: RefCell::new(vec![]), text: "# Report v2".to_string() };

        convert_if_needed(&source, "hash1", None, &converter).unwrap();
        convert_if_needed(&source, "hash2", Some("hash1"), &converter).unwrap();

        assert_eq!(converter.calls.borrow().len(), 2, "should reconvert on changed hash");
    }
}
