//! `mcp-guard` is a stdio gateway for one MCP server: it narrows the surface
//! the client can see and refuses blocked names locally.

pub mod guard;
pub mod policy;

pub use guard::{Client, Guard};
pub use policy::{Kind, Policy};
