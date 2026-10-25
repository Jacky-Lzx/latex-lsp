//! Language features and their request-specific caches and resolvers.

pub(crate) mod call_hierarchy;
pub mod code_lens;
pub mod colors;
pub(crate) mod command_docs;
pub(crate) mod completion_resolve;
pub mod features;
pub(crate) mod folding;
pub(crate) mod hook_arguments;
pub mod navigation;
pub mod path_completion;
pub mod semantic;
pub(crate) mod semantic_cache;
pub mod signature;
pub(crate) mod symbol_cache;
