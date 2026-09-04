//! rmcp stdio MCP server. Transport layer for the `csp::mcp` tool core (T021).
//!
//! Exposes the `search` and `find_related` tools over the Model Context Protocol
//! (stdio transport). The tool bodies delegate to the transport-agnostic,
//! unit-tested handlers in `csp::mcp`; this module only owns the rmcp wiring
//! (parameter schemas, the tool router, the server handler, and the runtime).

use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};
use tokio::sync::Mutex;

use csp::mcp::{
    find_related_tool, resolve_content_selection, search_tool, ContentSelection, IndexCache,
    SERVER_INSTRUCTIONS,
};
use csp::stats::default_stats_file;
use csp::types::ContentType;
use csp::utils::resolve_snippet_lines;

/// MCP default: signature + first body lines, enough to confirm a location
/// while spending far fewer tokens than the full chunk (semble#198).
fn default_max_snippet_lines() -> Option<i64> {
    Some(10)
}

/// Parameters for the `search` tool (mirrors the TS MCP tool's args).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language or code query.
    pub query: String,
    /// Optional git URL or local path to index on demand. Defaults to the
    /// server's pre-configured source.
    pub repo: Option<String>,
    /// Maximum number of results (default 5).
    pub top_k: Option<u32>,
    /// Lines of source per result. Default 10 = signature + first body lines,
    /// enough to confirm the location. 0 = file path and line range only. Pass
    /// `null` for the full chunk when the snippet lacks context.
    #[serde(default = "default_max_snippet_lines")]
    pub max_snippet_lines: Option<i64>,
    /// Content to search: `code`, `docs`, `config`, or `all`. Defaults to the
    /// MCP server's configured content (`--content`).
    pub content: Option<ContentSelection>,
}

/// Parameters for the `find_related` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindRelatedParams {
    /// Path to the file as stored in the index (use `file_path` from a search result).
    pub file_path: String,
    /// Line number within that file.
    pub line: i64,
    /// Optional git URL or local path to index on demand.
    pub repo: Option<String>,
    /// Maximum number of results (default 5).
    pub top_k: Option<u32>,
    /// Lines of source per result. Default 10 = signature + first body lines.
    /// 0 = location only. Pass `null` for the full chunk.
    #[serde(default = "default_max_snippet_lines")]
    pub max_snippet_lines: Option<i64>,
    /// Content containing the related file: `code`, `docs`, `config`, or
    /// `all`. Defaults to the MCP server's configured content (`--content`).
    pub content: Option<ContentSelection>,
}

/// MCP server holding the session index cache, the default source, and the
/// default content selection.
#[derive(Clone)]
pub struct CspMcpServer {
    cache: Arc<Mutex<IndexCache>>,
    default_source: Option<String>,
    default_ref: Option<String>,
    /// Content types indexed when a tool call omits `content` (the `--content`
    /// flag; code-only by default).
    default_content: Vec<ContentType>,
    /// Where token-savings telemetry is appended; `None` disables recording
    /// (used by tests so they don't touch the real `~/.csp/savings.jsonl`).
    stats_file: Option<std::path::PathBuf>,
    tool_router: ToolRouter<CspMcpServer>,
}

#[tool_router]
impl CspMcpServer {
    fn new(
        default_source: Option<String>,
        default_ref: Option<String>,
        content: Vec<ContentType>,
    ) -> Self {
        Self {
            cache: Arc::new(Mutex::new(IndexCache::new())),
            default_source,
            default_ref,
            default_content: content,
            stats_file: Some(default_stats_file()),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search a codebase with a natural-language or code query. Pass a git URL or local path as `repo` to index it on demand; indexes are cached for the session. Use this to find where something is implemented, understand a library, or locate related code."
    )]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = resolve_content_selection(p.content, &self.default_content);
        let mut cache = self.cache.lock().await;
        let out = search_tool(
            &mut cache,
            self.default_source.as_deref(),
            self.default_ref.as_deref(),
            &p.query,
            p.repo.as_deref(),
            &content,
            p.top_k.unwrap_or(5) as usize,
            resolve_snippet_lines(p.max_snippet_lines),
            self.stats_file.as_deref(),
        );
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    #[tool(
        description = "Find code chunks semantically similar to a specific location in a file. Use after `search` to explore related implementations or callers. Pass file_path and line from a prior search result."
    )]
    async fn find_related(
        &self,
        Parameters(p): Parameters<FindRelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = resolve_content_selection(p.content, &self.default_content);
        let mut cache = self.cache.lock().await;
        let out = find_related_tool(
            &mut cache,
            self.default_source.as_deref(),
            self.default_ref.as_deref(),
            &p.file_path,
            p.line,
            p.repo.as_deref(),
            &content,
            p.top_k.unwrap_or(5) as usize,
            resolve_snippet_lines(p.max_snippet_lines),
            self.stats_file.as_deref(),
        );
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }
}

// `router = self.tool_router` routes through the stored field (the default
// `Self::tool_router()` would rebuild the router on every call and leave the
// field unread).
#[tool_handler(router = self.tool_router)]
impl ServerHandler for CspMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(SERVER_INSTRUCTIONS.to_string())
    }
}

/// Start the MCP server on stdio and block until the client disconnects.
///
/// `default_source` is the source indexed when a tool call omits `repo`;
/// `default_ref` pins the git revision for that default source (the `--ref`
/// flag); `content` is the content-type filter applied when a tool call omits
/// its own `content` selection.
pub fn run_mcp(
    default_source: Option<String>,
    default_ref: Option<String>,
    content: Vec<ContentType>,
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let service = CspMcpServer::new(default_source, default_ref, content)
            .serve(stdio())
            .await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn search_params_deserialize_with_and_without_optionals() {
        // Only `query` is required; repo/top_k default to None.
        let minimal: SearchParams =
            serde_json::from_value(serde_json::json!({ "query": "greet" })).unwrap();
        assert_eq!(minimal.query, "greet");
        assert!(minimal.repo.is_none());
        assert!(minimal.top_k.is_none());
        assert!(minimal.content.is_none());
        // Absent max_snippet_lines → the MCP default of 10.
        assert_eq!(minimal.max_snippet_lines, Some(10));

        let full: SearchParams = serde_json::from_value(serde_json::json!({
            "query": "greet",
            "repo": "./x",
            "top_k": 3,
            "max_snippet_lines": 0
        }))
        .unwrap();
        assert_eq!(full.repo.as_deref(), Some("./x"));
        assert_eq!(full.top_k, Some(3));
        assert_eq!(full.max_snippet_lines, Some(0));

        // Explicit null → None (full chunk), distinct from the absent default.
        let nulled: SearchParams = serde_json::from_value(serde_json::json!({
            "query": "greet",
            "max_snippet_lines": null
        }))
        .unwrap();
        assert!(nulled.max_snippet_lines.is_none());
        assert!(resolve_snippet_lines(nulled.max_snippet_lines).is_none());
        assert_eq!(resolve_snippet_lines(Some(3)), Some(3));
        assert_eq!(resolve_snippet_lines(Some(-4)), Some(0));
    }

    #[test]
    fn find_related_params_deserialize() {
        let p: FindRelatedParams = serde_json::from_value(serde_json::json!({
            "file_path": "sample.ts",
            "line": 1
        }))
        .unwrap();
        assert_eq!(p.file_path, "sample.ts");
        assert_eq!(p.line, 1);
        assert!(p.repo.is_none());
        assert!(p.top_k.is_none());
        assert!(p.content.is_none());
    }

    #[test]
    fn params_accept_content_selection_and_reject_unknown() {
        let s: SearchParams =
            serde_json::from_value(serde_json::json!({ "query": "q", "content": "docs" })).unwrap();
        assert_eq!(s.content, Some(ContentSelection::Docs));
        let f: FindRelatedParams = serde_json::from_value(serde_json::json!({
            "file_path": "a.ts",
            "line": 1,
            "content": "all"
        }))
        .unwrap();
        assert_eq!(f.content, Some(ContentSelection::All));
        // Only the four documented values are accepted (no `tests`, no casing).
        assert!(serde_json::from_value::<SearchParams>(
            serde_json::json!({ "query": "q", "content": "tests" })
        )
        .is_err());
        assert!(serde_json::from_value::<SearchParams>(
            serde_json::json!({ "query": "q", "content": "Docs" })
        )
        .is_err());
    }

    #[test]
    fn tool_schemas_advertise_content_enum() {
        // The wire schema clients see must list the content selection so an
        // agent can discover the parameter without reading the README.
        let tools = CspMcpServer::tool_router().list_all();
        for tool in tools {
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            let content = &schema["properties"]["content"];
            assert!(
                content.is_object(),
                "{} schema lacks `content`: {schema}",
                tool.name
            );
            let text = content.to_string();
            for v in ["code", "docs", "config", "all"] {
                assert!(
                    text.contains(v),
                    "{} `content` lacks {v}: {text}",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn get_info_advertises_tools_and_instructions() {
        let server = CspMcpServer::new(None, None, vec![ContentType::Code]);
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
        assert_eq!(info.instructions.as_deref(), Some(SERVER_INSTRUCTIONS));
    }

    /// A temp dir with one indexable source file used as the default source.
    fn sample_source() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("sample.ts"),
            "export function greet(name: string) { return `hi ${name}` }\n",
        )
        .unwrap();
        dir
    }

    #[tokio::test]
    async fn search_tool_call_returns_json_payload() {
        let dir = sample_source();
        let mut server = CspMcpServer::new(
            Some(dir.path().to_string_lossy().into_owned()),
            None,
            vec![ContentType::Code],
        );
        // Don't append telemetry to the developer's real ~/.csp during tests.
        server.stats_file = None;
        let result = server
            .search(Parameters(SearchParams {
                query: "greet".to_string(),
                repo: None,
                top_k: Some(5),
                max_snippet_lines: None,
                content: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        // The tool body wraps the wire JSON from `search_tool` as text content.
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("results").is_some() || value.get("error").is_some());
    }

    /// Extract the text payload of a tool result as parsed JSON.
    fn payload(result: &CallToolResult) -> serde_json::Value {
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        serde_json::from_str(&text).unwrap()
    }

    /// File paths of every result in a search payload.
    fn result_paths(value: &serde_json::Value) -> Vec<String> {
        value["results"]
            .as_array()
            .map(|r| {
                r.iter()
                    .map(|e| e["file_path"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn search_tool_call_honors_per_call_content() {
        // A code-only server (the default) whose repo also holds a doc file.
        let dir = sample_source();
        fs::write(
            dir.path().join("README.md"),
            "# Guide\n\nHow to greet a user by name from the command line.\n",
        )
        .unwrap();
        let mut server = CspMcpServer::new(
            Some(dir.path().to_string_lossy().into_owned()),
            None,
            vec![ContentType::Code],
        );
        server.stats_file = None;
        let call = |content: Option<ContentSelection>| {
            server.search(Parameters(SearchParams {
                query: "greet".to_string(),
                repo: None,
                top_k: Some(5),
                max_snippet_lines: Some(0),
                content,
            }))
        };

        // Omitted → the server default (code): only sample.ts is indexed.
        let paths = result_paths(&payload(&call(None).await.unwrap()));
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|p| p == "sample.ts"), "{paths:?}");

        // `docs` on the same repo → a separate docs-only index.
        let paths = result_paths(&payload(&call(Some(ContentSelection::Docs)).await.unwrap()));
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|p| p == "README.md"), "{paths:?}");

        // `all` → both files are searchable in one index.
        let paths = result_paths(&payload(&call(Some(ContentSelection::All)).await.unwrap()));
        assert!(paths.iter().any(|p| p == "sample.ts"), "{paths:?}");
        assert!(paths.iter().any(|p| p == "README.md"), "{paths:?}");

        // Three distinct content selections → three cached entries.
        assert_eq!(server.cache.lock().await.size(), 3);
    }

    #[tokio::test]
    async fn find_related_tool_call_reports_missing_chunk() {
        let dir = sample_source();
        let server = CspMcpServer::new(
            Some(dir.path().to_string_lossy().into_owned()),
            None,
            vec![ContentType::Code],
        );
        let result = server
            .find_related(Parameters(FindRelatedParams {
                file_path: "nope.ts".to_string(),
                line: 1,
                repo: None,
                top_k: Some(5),
                max_snippet_lines: None,
                content: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        // No chunk at nope.ts:1 → an error payload (string), not a hard failure.
        assert!(text.contains("error") || text.contains("No "));
    }
}
