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

use csp::mcp::{find_related_tool, search_tool, IndexCache, SERVER_INSTRUCTIONS};
use csp::types::ContentType;

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
}

/// MCP server holding the session index cache and the default source.
#[derive(Clone)]
pub struct CspMcpServer {
    cache: Arc<Mutex<IndexCache>>,
    default_source: Option<String>,
    default_ref: Option<String>,
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
            cache: Arc::new(Mutex::new(IndexCache::new(content))),
            default_source,
            default_ref,
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
        let mut cache = self.cache.lock().await;
        let out = search_tool(
            &mut cache,
            self.default_source.as_deref(),
            self.default_ref.as_deref(),
            &p.query,
            p.repo.as_deref(),
            p.top_k.unwrap_or(5) as usize,
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
        let mut cache = self.cache.lock().await;
        let out = find_related_tool(
            &mut cache,
            self.default_source.as_deref(),
            self.default_ref.as_deref(),
            &p.file_path,
            p.line,
            p.repo.as_deref(),
            p.top_k.unwrap_or(5) as usize,
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
/// flag); `content` is the content-type filter applied when building indexes.
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
