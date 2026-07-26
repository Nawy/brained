use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};

use crate::types::FileKind;

#[derive(Debug, Clone, PartialEq)]
pub struct FileRecord {
    pub path: String,
    pub xxh3: String,
    pub mtime: f64,
    pub file_type: FileKind,
    pub source_kind: String,
    pub last_scanned: f64,
}

pub fn open_db(db_path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let conn = Connection::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            xxh3 TEXT NOT NULL,
            mtime REAL NOT NULL,
            type TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            last_scanned REAL NOT NULL
        )",
        [],
    ).with_context(|| "creating files table")?;
    Ok(conn)
}

pub fn get_file_record(conn: &Connection, path: &str) -> anyhow::Result<Option<FileRecord>> {
    conn.query_row(
        "SELECT path, xxh3, mtime, type, source_kind, last_scanned FROM files WHERE path = ?1",
        params![path],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
            ))
        },
    )
    .optional()
    .with_context(|| format!("fetching file record for {}", path))?
    .map(|(path, xxh3, mtime, kind, source_kind, last_scanned)| {
        Ok(FileRecord {
            path,
            xxh3,
            mtime,
            file_type: FileKind::parse(&kind)?,
            source_kind,
            last_scanned,
        })
    })
    .transpose()
}

pub fn upsert_file_record(conn: &Connection, record: &FileRecord) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO files (path, xxh3, mtime, type, source_kind, last_scanned)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(path) DO UPDATE SET
            xxh3 = excluded.xxh3,
            mtime = excluded.mtime,
            type = excluded.type,
            source_kind = excluded.source_kind,
            last_scanned = excluded.last_scanned",
        params![
            record.path,
            record.xxh3,
            record.mtime,
            record.file_type.as_str(),
            record.source_kind,
            record.last_scanned,
        ],
    ).with_context(|| format!("upserting file record for {}", record.path))?;
    Ok(())
}

pub fn delete_file_record(conn: &Connection, path: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM files WHERE path = ?1", params![path])
        .with_context(|| format!("deleting file record for {}", path))?;
    Ok(())
}

pub fn list_all_paths(conn: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT path FROM files")
        .with_context(|| "preparing statement to list all file paths")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))
        .with_context(|| "querying all file paths")?;
    let mut paths = HashSet::new();
    for row in rows {
        paths.insert(row.with_context(|| "reading file path row")?);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileKind;

    fn sample(path: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            xxh3: "abc123".to_string(),
            mtime: 1000.0,
            file_type: FileKind::Knowledge,
            source_kind: "md".to_string(),
            last_scanned: 2000.0,
        }
    }

    #[test]
    fn round_trips_a_record() {
        let conn = open_db(&tempfile::tempdir().unwrap().path().join("state.db")).unwrap();
        upsert_file_record(&conn, &sample("docs/a.md")).unwrap();
        let fetched = get_file_record(&conn, "docs/a.md").unwrap().unwrap();
        assert_eq!(fetched, sample("docs/a.md"));
    }

    #[test]
    fn missing_record_is_none() {
        let conn = open_db(&tempfile::tempdir().unwrap().path().join("state.db")).unwrap();
        assert!(get_file_record(&conn, "nope.md").unwrap().is_none());
    }

    #[test]
    fn upsert_overwrites_existing() {
        let conn = open_db(&tempfile::tempdir().unwrap().path().join("state.db")).unwrap();
        upsert_file_record(&conn, &sample("docs/a.md")).unwrap();
        let mut updated = sample("docs/a.md");
        updated.xxh3 = "changed".to_string();
        upsert_file_record(&conn, &updated).unwrap();
        assert_eq!(get_file_record(&conn, "docs/a.md").unwrap().unwrap().xxh3, "changed");
    }

    #[test]
    fn delete_removes_record() {
        let conn = open_db(&tempfile::tempdir().unwrap().path().join("state.db")).unwrap();
        upsert_file_record(&conn, &sample("docs/a.md")).unwrap();
        delete_file_record(&conn, "docs/a.md").unwrap();
        assert!(get_file_record(&conn, "docs/a.md").unwrap().is_none());
    }

    #[test]
    fn list_all_paths_returns_every_path() {
        let conn = open_db(&tempfile::tempdir().unwrap().path().join("state.db")).unwrap();
        upsert_file_record(&conn, &sample("docs/a.md")).unwrap();
        upsert_file_record(&conn, &sample("docs/b.md")).unwrap();
        let paths = list_all_paths(&conn).unwrap();
        assert_eq!(paths, ["docs/a.md".to_string(), "docs/b.md".to_string()].into_iter().collect::<HashSet<_>>());
    }

    #[test]
    fn open_db_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested").join("state.db");
        assert!(open_db(&nested).is_ok());
        assert!(nested.exists());
    }
}
