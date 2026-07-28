use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use tokio::sync::RwLock;

use crate::domain::Domain;
use crate::scan::SharedState;
use crate::types::FileKind;
use crate::vector_store::ChunkResult;

pub fn format_results(results: &[ChunkResult]) -> String {
    if results.is_empty() {
        return "No results.".to_string();
    }
    results
        .iter()
        .map(|r| format!("@{} [{}/{}]\n{}", r.path, r.domain.as_str(), r.file_type.as_str(), r.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn parse_domain_filter(domain_str: &str) -> anyhow::Result<Option<Domain>> {
    match domain_str {
        "both" => Ok(None),
        other => Domain::parse(other).map(Some),
    }
}

fn parse_kind_filter(kind_str: &str) -> anyhow::Result<Option<FileKind>> {
    match kind_str {
        "both" => Ok(None),
        other => FileKind::parse(other).map(Some),
    }
}

pub async fn search_impl(
    query: &str,
    domain_str: &str,
    kind_str: &str,
    top_k: usize,
    state: &SharedState,
) -> anyhow::Result<String> {
    let domain_filter = parse_domain_filter(domain_str)?;
    let kind_filter = parse_kind_filter(kind_str)?;
    // embed_query needs &mut self (fastembed's TextEmbedding::embed does — see Task 9/13), so
    // it goes through the embedder's own inner Mutex rather than the outer RwLock read guard.
    let query_vector = state.embedder.lock().await.embed_query(query)?;
    let results = state.vector_store.query_chunks(&query_vector, domain_filter, kind_filter, top_k).await?;
    Ok(format_results(&results))
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchKnowledgeRequest {
    #[schemars(description = "a short, specific phrase or sentence to search for")]
    pub query: String,
    #[schemars(description = "\"business\", \"tech\", or \"both\" (default \"both\")")]
    #[serde(default = "default_domain")]
    pub domain: String,
    #[schemars(description = "\"knowledge\", \"work\", or \"both\" (default \"both\")")]
    #[serde(default = "default_kind")]
    pub kind: String,
    #[schemars(description = "how many results to return (default 5)")]
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExtractAnomalyRequest {
    #[schemars(description = "same short phrase/sentence semantics as search_knowledge's query")]
    pub context_query: String,
    #[schemars(description = "\"business\", \"tech\", or \"both\" (default \"both\")")]
    #[serde(default = "default_domain")]
    pub domain: String,
    #[schemars(description = "\"knowledge\", \"work\", or \"both\" (default \"both\")")]
    #[serde(default = "default_kind")]
    pub kind: String,
    #[schemars(description = "how many candidates to compare (default 12)")]
    #[serde(default = "default_anomaly_top_k")]
    pub top_k: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteWorkFileRequest {
    #[schemars(description = "\"business\" or \"tech\"")]
    pub domain: String,
    #[schemars(description = "path relative to <domain>/work/, e.g. \"debug-notes/2026-07-27.md\" (never targets knowledge/)")]
    pub path: String,
    #[schemars(description = "the file's full new content")]
    pub content: String,
}

fn default_domain() -> String {
    "both".to_string()
}
fn default_kind() -> String {
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

impl BrdMcpServer {
    async fn try_lock_domain(&self, domain: Domain) -> String {
        let state = self.state.read().await;
        let held_by_us = match domain {
            Domain::Tech => &state.tech_locked_by_us,
            Domain::Business => &state.business_locked_by_us,
        };
        match crate::lock::try_lock(&state.root, domain, held_by_us, std::process::id(), &crate::lock::real_is_alive) {
            Ok(true) => format!("{} is now locked by this session.", domain.as_str()),
            Ok(false) => format!("{} is locked by another session — wait and retry, or ask the human if this was a read.", domain.as_str()),
            Err(e) => format!("lock{} failed: {e}", domain.as_str()),
        }
    }

    async fn unlock_domain(&self, domain: Domain) -> String {
        let state = self.state.read().await;
        let held_by_us = match domain {
            Domain::Tech => &state.tech_locked_by_us,
            Domain::Business => &state.business_locked_by_us,
        };
        match crate::lock::unlock(&state.root, domain, held_by_us) {
            Ok(true) => format!("{} unlocked.", domain.as_str()),
            Ok(false) => format!("{} was not locked by this session — nothing to unlock.", domain.as_str()),
            Err(e) => format!("unlock{} failed: {e}", domain.as_str()),
        }
    }
}

#[tool_router(server_handler)]
impl BrdMcpServer {
    #[tool(description = "Search indexed knowledge/work files for chunks relevant to query.")]
    async fn search_knowledge(
        &self,
        Parameters(req): Parameters<SearchKnowledgeRequest>,
    ) -> String {
        let state = self.state.read().await;
        search_impl(&req.query, &req.domain, &req.kind, req.top_k, &state)
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
        search_impl(&req.context_query, &req.domain, &req.kind, req.top_k, &state)
            .await
            .unwrap_or_else(|e| format!("extract_anomaly failed: {e}"))
    }

    #[tool(
        description = "Write (create or overwrite) a file under <domain>/work/. Rejects paths that \
        escape work/ or target a knowledge/ folder. Requires holding domain's lock first (call \
        locktech/lockbusiness) — if this returns a \"not locked\" error, acquire the lock and retry."
    )]
    async fn write_work_file(
        &self,
        Parameters(req): Parameters<WriteWorkFileRequest>,
    ) -> String {
        let state = self.state.read().await;
        let Ok(domain) = Domain::parse(&req.domain) else {
            return format!("write_work_file failed: unknown domain {:?}, expected \"business\" or \"tech\"", req.domain);
        };
        crate::work_file::write_work_file(&state, domain, &req.path, &req.content)
            .await
            .map(|()| format!("Wrote {}/work/{}.", domain.as_str(), req.path))
            .unwrap_or_else(|e| format!("write_work_file failed: {e}"))
    }

    #[tool(description = "Try to acquire the tech domain's lock for this session. Returns whether it \
        succeeded — false means another session already holds it (wait and retry for a planned write; \
        ask the human whether to proceed read-only or wait for a planned read).")]
    async fn locktech(&self) -> String {
        self.try_lock_domain(Domain::Tech).await
    }

    #[tool(description = "Release this session's hold on the tech domain's lock.")]
    async fn unlocktech(&self) -> String {
        self.unlock_domain(Domain::Tech).await
    }

    #[tool(description = "Try to acquire the business domain's lock for this session. Returns whether \
        it succeeded — false means another session already holds it (wait and retry for a planned \
        write; ask the human whether to proceed read-only or wait for a planned read).")]
    async fn lockbusiness(&self) -> String {
        self.try_lock_domain(Domain::Business).await
    }

    #[tool(description = "Release this session's hold on the business domain's lock.")]
    async fn unlockbusiness(&self) -> String {
        self.unlock_domain(Domain::Business).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Domain;
    use crate::types::FileKind;
    use crate::vector_store::ChunkResult;

    #[test]
    fn formats_no_results() {
        assert_eq!(format_results(&[]), "No results.");
    }

    #[test]
    fn formats_one_result() {
        let results = vec![ChunkResult { path: "a.md".to_string(), domain: Domain::Business, file_type: FileKind::Knowledge, text: "hello".to_string() }];
        assert_eq!(format_results(&results), "@a.md [business/knowledge]\nhello");
    }

    #[test]
    fn separates_multiple_results_with_blank_line() {
        let results = vec![
            ChunkResult { path: "a.md".to_string(), domain: Domain::Business, file_type: FileKind::Knowledge, text: "hello".to_string() },
            ChunkResult { path: "b.md".to_string(), domain: Domain::Tech, file_type: FileKind::Work, text: "world".to_string() },
        ];
        assert_eq!(format_results(&results), "@a.md [business/knowledge]\nhello\n\n@b.md [tech/work]\nworld");
    }
}
