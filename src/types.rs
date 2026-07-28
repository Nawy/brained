#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Knowledge,
    Work,
}

impl FileKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileKind::Knowledge => "knowledge",
            FileKind::Work => "work",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<FileKind> {
        match s {
            "knowledge" => Ok(FileKind::Knowledge),
            "work" => Ok(FileKind::Work),
            other => anyhow::bail!("unknown file kind {other:?}, expected \"knowledge\" or \"work\""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_str() {
        assert_eq!(FileKind::parse("knowledge").unwrap(), FileKind::Knowledge);
        assert_eq!(FileKind::parse("work").unwrap(), FileKind::Work);
        assert_eq!(FileKind::Knowledge.as_str(), "knowledge");
        assert!(FileKind::parse("bogus").is_err());
    }
}
