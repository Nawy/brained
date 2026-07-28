use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

use crate::domain::Domain;
use crate::types::FileKind;

const TABLE_NAME: &str = "chunks";

pub struct ChunkResult {
    pub path: String,
    pub domain: Domain,
    pub file_type: FileKind,
    pub text: String,
}

pub struct VectorStore {
    table: lancedb::Table,
    dim: i32,
}

fn schema(dim: i32) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("domain", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
            true,
        ),
    ]))
}

fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

impl VectorStore {
    pub async fn open(db_dir: &Path, dim: i32) -> anyhow::Result<Self> {
        let db = lancedb::connect(db_dir.to_str().context("db_dir must be valid UTF-8")?)
            .execute()
            .await
            .context("connecting to LanceDB")?;

        let table = match db.open_table(TABLE_NAME).execute().await {
            Ok(t) => t,
            Err(lancedb::Error::TableNotFound { .. }) => {
                let empty_batch = RecordBatch::try_new(
                    schema(dim),
                    vec![
                        Arc::new(StringArray::from(Vec::<&str>::new())),
                        Arc::new(Int32Array::from(Vec::<i32>::new())),
                        Arc::new(StringArray::from(Vec::<&str>::new())),
                        Arc::new(StringArray::from(Vec::<&str>::new())),
                        Arc::new(StringArray::from(Vec::<&str>::new())),
                        Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                            Vec::<Option<Vec<Option<f32>>>>::new(),
                            dim,
                        )),
                    ],
                )?;
                db.create_table(TABLE_NAME, empty_batch)
                    .execute()
                    .await
                    .context("creating LanceDB table")?
            }
            Err(e) => return Err(e).context("opening LanceDB table"),
        };

        Ok(VectorStore { table, dim })
    }

    pub async fn add_chunks(
        &self,
        path: &str,
        domain: Domain,
        file_type: FileKind,
        chunks: &[String],
        vectors: &[Vec<f32>],
    ) -> anyhow::Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        anyhow::ensure!(chunks.len() == vectors.len(), "chunks/vectors length mismatch");

        let n = chunks.len();
        let batch = RecordBatch::try_new(
            schema(self.dim),
            vec![
                Arc::new(StringArray::from(vec![path; n])),
                Arc::new(Int32Array::from((0..n as i32).collect::<Vec<_>>())),
                Arc::new(StringArray::from(vec![domain.as_str(); n])),
                Arc::new(StringArray::from(vec![file_type.as_str(); n])),
                Arc::new(StringArray::from(chunks.to_vec())),
                Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                    vectors.iter().map(|v| Some(v.iter().map(|x| Some(*x)).collect::<Vec<_>>())),
                    self.dim,
                )),
            ],
        )?;
        self.table.add(batch).execute().await.context("adding chunks to LanceDB")?;
        Ok(())
    }

    pub async fn delete_chunks_for_path(&self, path: &str) -> anyhow::Result<()> {
        self.table
            .delete(format!("path = '{}'", escape_sql_string(path)).as_str())
            .await
            .context("deleting chunks from LanceDB")?;
        Ok(())
    }

    pub async fn query_chunks(
        &self,
        query_vector: &[f32],
        domain_filter: Option<Domain>,
        kind_filter: Option<FileKind>,
        top_k: usize,
    ) -> anyhow::Result<Vec<ChunkResult>> {
        let mut query = self.table.query().limit(top_k).nearest_to(query_vector)?;
        let mut conditions = Vec::new();
        if let Some(domain) = domain_filter {
            conditions.push(format!("domain = '{}'", domain.as_str()));
        }
        if let Some(kind) = kind_filter {
            conditions.push(format!("kind = '{}'", kind.as_str()));
        }
        if !conditions.is_empty() {
            query = query.only_if(conditions.join(" AND "));
        }
        let batches: Vec<RecordBatch> = query.execute().await.context("running LanceDB search")?.try_collect().await?;

        let mut results = Vec::new();
        for batch in batches {
            let paths = batch.column_by_name("path").context("missing path column")?
                .as_any().downcast_ref::<StringArray>().context("path column has wrong type")?;
            let domains = batch.column_by_name("domain").context("missing domain column")?
                .as_any().downcast_ref::<StringArray>().context("domain column has wrong type")?;
            let kinds = batch.column_by_name("kind").context("missing kind column")?
                .as_any().downcast_ref::<StringArray>().context("kind column has wrong type")?;
            let texts = batch.column_by_name("text").context("missing text column")?
                .as_any().downcast_ref::<StringArray>().context("text column has wrong type")?;
            for i in 0..batch.num_rows() {
                results.push(ChunkResult {
                    path: paths.value(i).to_string(),
                    domain: Domain::parse(domains.value(i))?,
                    file_type: FileKind::parse(kinds.value(i))?,
                    text: texts.value(i).to_string(),
                });
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Domain;
    use crate::types::FileKind;

    const TEST_DIM: i32 = 4;

    fn vec4(fill: f32) -> Vec<f32> {
        vec![fill; TEST_DIM as usize]
    }

    #[tokio::test]
    async fn add_then_query_returns_matching_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        store
            .add_chunks("a.md", Domain::Tech, FileKind::Knowledge, &["hello chunk".to_string()], &[vec4(1.0)])
            .await
            .unwrap();

        let results = store.query_chunks(&vec4(1.0), None, None, 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "a.md");
        assert_eq!(results[0].text, "hello chunk");
        assert!(matches!(results[0].domain, Domain::Tech));
        assert!(matches!(results[0].file_type, FileKind::Knowledge));
    }

    #[tokio::test]
    async fn query_respects_kind_filter() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        store.add_chunks("k.md", Domain::Tech, FileKind::Knowledge, &["k text".to_string()], &[vec4(1.0)]).await.unwrap();
        store.add_chunks("r.md", Domain::Tech, FileKind::Work, &["r text".to_string()], &[vec4(1.0)]).await.unwrap();

        let results = store.query_chunks(&vec4(1.0), None, Some(FileKind::Work), 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "r.md");
    }

    #[tokio::test]
    async fn query_respects_domain_filter() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        store.add_chunks("b.md", Domain::Business, FileKind::Knowledge, &["b text".to_string()], &[vec4(1.0)]).await.unwrap();
        store.add_chunks("t.md", Domain::Tech, FileKind::Knowledge, &["t text".to_string()], &[vec4(1.0)]).await.unwrap();

        let results = store.query_chunks(&vec4(1.0), Some(Domain::Business), None, 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "b.md");
    }

    #[tokio::test]
    async fn query_respects_domain_and_kind_filter_together() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        store.add_chunks("bk.md", Domain::Business, FileKind::Knowledge, &["bk text".to_string()], &[vec4(1.0)]).await.unwrap();
        store.add_chunks("bw.md", Domain::Business, FileKind::Work, &["bw text".to_string()], &[vec4(1.0)]).await.unwrap();
        store.add_chunks("tk.md", Domain::Tech, FileKind::Knowledge, &["tk text".to_string()], &[vec4(1.0)]).await.unwrap();

        let results = store
            .query_chunks(&vec4(1.0), Some(Domain::Business), Some(FileKind::Knowledge), 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "bk.md");
    }

    #[tokio::test]
    async fn query_respects_top_k() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        for i in 0..5 {
            store
                .add_chunks(&format!("f{i}.md"), Domain::Tech, FileKind::Knowledge, &[format!("chunk {i}")], &[vec4(1.0)])
                .await
                .unwrap();
        }
        let results = store.query_chunks(&vec4(1.0), None, None, 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn delete_chunks_for_path_removes_only_that_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        store.add_chunks("a.md", Domain::Tech, FileKind::Knowledge, &["a text".to_string()], &[vec4(1.0)]).await.unwrap();
        store.add_chunks("b.md", Domain::Tech, FileKind::Knowledge, &["b text".to_string()], &[vec4(1.0)]).await.unwrap();

        store.delete_chunks_for_path("a.md").await.unwrap();

        let results = store.query_chunks(&vec4(1.0), None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "b.md");
    }

    #[tokio::test]
    async fn reopening_an_existing_store_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
            store.add_chunks("a.md", Domain::Tech, FileKind::Knowledge, &["a text".to_string()], &[vec4(1.0)]).await.unwrap();
        }
        let store = VectorStore::open(dir.path(), TEST_DIM).await.unwrap();
        let results = store.query_chunks(&vec4(1.0), None, None, 10).await.unwrap();
        assert_eq!(results.len(), 1);
    }
}
