use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use tokio::sync::RwLock;

use crate::scan::SharedState;
use crate::types::FileKind;
use crate::vector_store::ChunkResult;

pub fn format_results(results: &[ChunkResult]) -> String {
    if results.is_empty() {
        return "No results.".to_string();
    }
    results
        .iter()
        .map(|r| format!("@{} [{}]\n{}", r.path, r.file_type.as_str(), r.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn parse_type_filter(type_str: &str) -> anyhow::Result<Option<FileKind>> {
    match type_str {
        "both" => Ok(None),
        other => FileKind::parse(other).map(Some),
    }
}

pub async fn search_impl(query: &str, type_str: &str, top_k: usize, state: &SharedState) -> anyhow::Result<String> {
    let type_filter = parse_type_filter(type_str)?;
    // embed_query needs &mut self (fastembed's TextEmbedding::embed does — see Task 9/13), so
    // it goes through the embedder's own inner Mutex rather than the outer RwLock read guard.
    let query_vector = state.embedder.lock().await.embed_query(query)?;
    let results = state.vector_store.query_chunks(&query_vector, type_filter, top_k).await?;
    Ok(format_results(&results))
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchKnowledgeRequest {
    #[schemars(description = "a short, specific phrase or sentence to search for")]
    pub query: String,
    #[schemars(description = "\"knowledge\", \"research\", or \"both\" (default \"both\")")]
    #[serde(default = "default_type")]
    pub r#type: String,
    #[schemars(description = "how many results to return (default 5)")]
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExtractAnomalyRequest {
    #[schemars(description = "same short phrase/sentence semantics as search_knowledge's query")]
    pub context_query: String,
    #[schemars(description = "\"knowledge\", \"research\", or \"both\" (default \"both\")")]
    #[serde(default = "default_type")]
    pub r#type: String,
    #[schemars(description = "how many candidates to compare (default 12)")]
    #[serde(default = "default_anomaly_top_k")]
    pub top_k: usize,
}

fn default_type() -> String {
    "both".to_string()
}
fn default_top_k() -> usize {
    5
}
fn default_anomaly_top_k() -> usize {
    12
}

#[derive(Clone)]
pub struct BrdMcpServer {
    pub state: Arc<RwLock<SharedState>>,
}

#[tool_router(server_handler)]
impl BrdMcpServer {
    #[tool(description = "Search indexed knowledge/research files for chunks relevant to query.")]
    async fn search_knowledge(
        &self,
        Parameters(req): Parameters<SearchKnowledgeRequest>,
    ) -> String {
        let state = self.state.read().await;
        search_impl(&req.query, &req.r#type, req.top_k, &state)
            .await
            .unwrap_or_else(|e| format!("search_knowledge failed: {e}"))
    }

    #[tool(
        description = "Retrieve chunks for context_query and return them so the calling agent can find \
        the one that most strongly contradicts or diverges from the rest — not an average or summary \
        of all results, the specific outlier. Use this instead of search_knowledge when hunting for \
        contradictions, outliers, or dissenting opinions rather than doing a normal lookup."
    )]
    async fn extract_anomaly(
        &self,
        Parameters(req): Parameters<ExtractAnomalyRequest>,
    ) -> String {
        let state = self.state.read().await;
        search_impl(&req.context_query, &req.r#type, req.top_k, &state)
            .await
            .unwrap_or_else(|e| format!("extract_anomaly failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileKind;
    use crate::vector_store::ChunkResult;

    #[test]
    fn formats_no_results() {
        assert_eq!(format_results(&[]), "No results.");
    }

    #[test]
    fn formats_one_result() {
        let results = vec![ChunkResult { path: "a.md".to_string(), file_type: FileKind::Knowledge, text: "hello".to_string() }];
        assert_eq!(format_results(&results), "@a.md [knowledge]\nhello");
    }

    #[test]
    fn separates_multiple_results_with_blank_line() {
        let results = vec![
            ChunkResult { path: "a.md".to_string(), file_type: FileKind::Knowledge, text: "hello".to_string() },
            ChunkResult { path: "b.md".to_string(), file_type: FileKind::Research, text: "world".to_string() },
        ];
        assert_eq!(format_results(&results), "@a.md [knowledge]\nhello\n\n@b.md [research]\nworld");
    }
}
