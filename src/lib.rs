//! LaTeX language server with modules organized by responsibility.

mod bibliography;
pub mod compilation;
pub mod config;
pub mod diagnostics;
pub mod editing;
mod formatting;
pub mod language;
pub mod parser;
pub mod project;
pub mod rpc;
pub mod server;
mod symbols;
pub mod text;
