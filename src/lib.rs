//! LaTeX language server with modules organized by responsibility.

mod bibliography;
mod compilation;
pub mod config;
mod diagnostics;
pub mod editing;
mod formatting;
mod language;
mod parser;
mod project;
mod server;
mod symbols;
pub mod text;

// Preserve the public module paths used by clients and integration tests.
pub use compilation::build;
pub use compilation::build_log;
pub use compilation::recorder;
pub use compilation::synctex;
pub use diagnostics::diagnostic;
pub use editing::file_operations;
pub use language::code_lens;
pub use language::colors;
pub use language::features;
pub use language::navigation;
pub use language::path_completion;
pub use language::semantic;
pub use language::signature;
pub use parser::syntax;
pub use project::catalog;
pub use project::distribution;
pub use project::graphics;
pub use project::workspace;
pub use server::dispatch;
pub use server::Backend;
