use lsp_types::{CompletionItem, Documentation};

pub fn preview(item: &CompletionItem) -> &str {
    match &item.documentation {
        Some(Documentation::MarkupContent(content)) => &content.value,
        Some(Documentation::String(content)) => content,
        None => item.detail.as_deref().unwrap_or_default(),
    }
}
