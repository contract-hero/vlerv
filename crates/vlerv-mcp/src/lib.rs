// vlerv-mcp — an MCP server that lets a coding agent send artifacts to the
// Vlervtifacts devices a person actually reads them on.
//
// The agent produces a report, a chart, an HTML explainer; the person wants it
// on their phone or their other Mac. This server is the bridge: it speaks MCP
// over stdio to the agent, and iroh to the devices, with no service in the
// middle and nothing uploaded anywhere.
//
// It is a PEER, not a client. It holds its own ed25519 identity and its own
// `peers.json` under `~/Library/Application Support/Vlerv/mcp/`, so pairing a
// device with this server is a separate act from pairing it with the desktop
// app — revoking one leaves the other alone. Everything networked comes from
// the `vlerv-remote` crate, unchanged: the same request gate, the same RootSet
// path gate, the same peer-locked grants, the same BLAKE3 verification.
//
// Layout:
//   * `args`    — the tool schemas and the pure argument validation;
//   * `devices` — turning a model's free-text device name into one peer;
//   * `core`    — the state and the tool logic, with no MCP framing in it;
//   * `server`  — the rmcp handler that wraps `core` in eight tools.

pub mod args;
pub mod core;
pub mod devices;
pub mod server;

pub use core::McpCore;
pub use server::VlervMcp;
