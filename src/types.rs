#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Knowledge,
    Research,
}

impl FileKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileKind::Knowledge => "knowledge",
            FileKind::Research => "research",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<FileKind> {
        match s {
            "knowledge" => Ok(FileKind::Knowledge),
            "research" => Ok(FileKind::Research),
            other => anyhow::bail!("unknown file kind {other:?}, expected \"knowledge\" or \"research\""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_str() {
        assert_eq!(FileKind::parse("knowledge").unwrap(), FileKind::Knowledge);
        assert_eq!(FileKind::parse("research").unwrap(), FileKind::Research);
        assert_eq!(FileKind::Knowledge.as_str(), "knowledge");
        assert!(FileKind::parse("bogus").is_err());
    }
}
