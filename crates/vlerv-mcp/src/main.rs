// The `vlerv-mcp` binary: one MCP server on stdio.
//
// STDOUT IS THE PROTOCOL. Everything this process wants to say to a human goes
// to stderr — a stray `println!` anywhere in the tree would corrupt the
// JSON-RPC stream and the client would drop the connection.
//
// Nothing binds a socket here. The iroh endpoint boots inside the first tool
// call that needs it, so an agent that registers this server and never sends a
// file makes no network connections at all.

use std::sync::Arc;

use rmcp::transport::stdio;
use rmcp::ServiceExt;
use vlerv_mcp::{McpCore, VlervMcp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let core = Arc::new(McpCore::from_env());
    eprintln!(
        "vlerv-mcp: {} — node {}",
        core.device(),
        core.node_id().unwrap_or_else(|e| format!("<no identity: {e}>"))
    );

    let service = VlervMcp::new(core).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
