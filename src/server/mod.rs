//! LSP handlers, ordered dispatch and external task lifecycle.

mod backend;
pub mod dispatch;
pub(crate) mod output;
pub(crate) mod tasks;

pub use backend::Backend;
