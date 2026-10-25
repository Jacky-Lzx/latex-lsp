//! Source-local outlines shared by document and workspace symbol requests.
use crate::project::workspace::Document;
use lsp_types::DocumentSymbol;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
static CACHED_BYTES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub(crate) struct Snapshot {
    pub(crate) symbols: Vec<DocumentSymbol>,
    retained_bytes: usize,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        CACHED_BYTES.fetch_sub(self.retained_bytes, Ordering::Relaxed);
    }
}

fn allocated_bytes(symbols: &Vec<DocumentSymbol>) -> usize {
    symbols.capacity() * std::mem::size_of::<DocumentSymbol>()
        + symbols
            .iter()
            .map(|symbol| {
                symbol.name.capacity()
                    + symbol.detail.as_ref().map_or(0, String::capacity)
                    + symbol.tags.as_ref().map_or(0, |tags| {
                        tags.capacity() * std::mem::size_of::<lsp_types::SymbolTag>()
                    })
                    + symbol.children.as_ref().map_or(0, allocated_bytes)
            })
            .sum::<usize>()
}

pub(crate) fn outline(doc: &Document) -> Arc<Snapshot> {
    if let Some(snapshot) = doc.outline.get() {
        return snapshot.clone();
    }
    let symbols = super::features::uncached_symbols(doc);
    let bytes = allocated_bytes(&symbols)
        + std::mem::size_of::<Snapshot>()
        + 2 * std::mem::size_of::<usize>();
    let retained = CACHED_BYTES
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current
                .checked_add(bytes)
                .filter(|total| *total <= MAX_BYTES)
        })
        .is_ok();
    let snapshot = Arc::new(Snapshot {
        symbols,
        retained_bytes: if retained { bytes } else { 0 },
    });
    if retained {
        // A concurrent reader may win. The unused snapshot releases its own
        // reservation when this query ends; no shared accounting is lost.
        let _ = doc.outline.set(snapshot.clone());
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Url;

    #[test]
    fn oversized_outlines_still_return_complete_symbols_without_being_retained() {
        let uri = Url::parse("untitled:large-outline.tex").unwrap();
        let name = "x".repeat(MAX_BYTES);
        let doc = Document::new(&uri, format!("\\section{{{name}}}"), Some(1));
        let snapshot = outline(&doc);
        assert!(doc.outline.get().is_none());
        assert_eq!(snapshot.retained_bytes, 0);
        assert_eq!(snapshot.symbols.len(), 1);
        assert_eq!(snapshot.symbols[0].name, name);
        assert_eq!(snapshot.symbols[0].selection_range.start.character, 9);
        assert_eq!(
            snapshot.symbols[0].selection_range.end.character,
            (MAX_BYTES + 9) as u32
        );
    }
}
