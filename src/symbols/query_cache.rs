//! Owned symbol analyses shared only during one immutable workspace query.
use super::{label_declarations, named_references};
use crate::project::workspace::Workspace;
use lsp_types::Url;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_ENTRIES: usize = 4;
const MAX_BYTES: usize = 8 * 1024 * 1024;
type Named = BTreeMap<(usize, Url), Arc<named_references::Index>>;
type Labels = BTreeMap<(usize, BTreeSet<Url>), Arc<label_declarations::Index>>;

#[derive(Default)]
struct Cache {
    named: Named,
    labels: Labels,
    bytes: usize,
}

thread_local! {
    static QUERY: RefCell<Option<Cache>> = const { RefCell::new(None) };
}

struct Restore(Option<Cache>);
impl Drop for Restore {
    fn drop(&mut self) {
        QUERY.with(|query| *query.borrow_mut() = self.0.take());
    }
}

pub(crate) fn with_cache<T>(work: impl FnOnce() -> T) -> T {
    let _restore = Restore(QUERY.with(|query| query.replace(Some(Cache::default()))));
    crate::project::source_inputs::with_query_cache(work)
}

fn shared<K: Ord, V>(
    key: K,
    select: impl Fn(&mut Cache) -> &mut BTreeMap<K, Arc<V>>,
    build: impl FnOnce() -> V,
    bytes: impl FnOnce(&V) -> usize,
) -> Arc<V> {
    if let Some(index) = QUERY.with(|query| {
        query
            .borrow_mut()
            .as_mut()
            .and_then(|cache| select(cache).get(&key).cloned())
    }) {
        return index;
    }
    // Analysing a scope can recursively query other scopes. Never retain a
    // RefCell borrow while computing, and only cache immutable, owned results.
    let index = Arc::new(build());
    QUERY.with(|query| {
        if let Some(cache) = query.borrow_mut().as_mut() {
            let bytes = bytes(&index);
            if select(cache).len() < MAX_ENTRIES && bytes <= MAX_BYTES - cache.bytes {
                select(cache).insert(key, index.clone());
                cache.bytes += bytes;
            }
        }
    });
    index
}

pub(crate) fn named(
    workspace: &Workspace,
    origin: &Url,
    build: impl FnOnce() -> named_references::Index,
) -> Arc<named_references::Index> {
    if QUERY.with(|query| query.borrow().is_none()) {
        return Arc::new(build());
    }
    shared(
        (workspace as *const Workspace as usize, origin.clone()),
        |cache| &mut cache.named,
        build,
        |index| {
            index
                .cache_bytes()
                .saturating_add(origin.as_str().len() + 64)
        },
    )
}

pub(crate) fn labels(
    workspace: &Workspace,
    scope: &BTreeSet<Url>,
    build: impl FnOnce() -> label_declarations::Index,
) -> Arc<label_declarations::Index> {
    if QUERY.with(|query| query.borrow().is_none()) {
        return Arc::new(build());
    }
    let key_bytes = scope
        .iter()
        .map(|uri| uri.as_str().len() + 64)
        .sum::<usize>();
    shared(
        (workspace as *const Workspace as usize, scope.clone()),
        |cache| &mut cache.labels,
        build,
        |index| index.cache_bytes().saturating_add(key_bytes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::workspace::Document;

    fn document(workspace: &mut Workspace, uri: &Url, source: &str) {
        workspace
            .documents
            .insert(uri.clone(), Document::new(uri, source.into(), Some(1)));
    }

    #[test]
    fn named_queries_share_work_but_refresh_meanings_and_restore_nested_scopes() {
        let uri = Url::parse("untitled:query.tex").unwrap();
        let source = r"\label{key}\ref{key}";
        let mut workspace = Workspace::default();
        document(&mut workspace, &uri, source);
        let at = source.rfind("key").unwrap();
        let retained = with_cache(|| {
            let index = named_references::Index::new(&workspace, &uri);
            assert!(index.target(&uri, at).is_some());
            assert!(Arc::ptr_eq(
                &index,
                &named_references::Index::new(&workspace, &uri)
            ));
            let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_cache(|| {
                    assert!(!Arc::ptr_eq(
                        &index,
                        &named_references::Index::new(&workspace, &uri)
                    ));
                    panic!("abandon nested analysis");
                })
            }));
            assert!(nested.is_err());
            assert!(Arc::ptr_eq(
                &index,
                &named_references::Index::new(&workspace, &uri)
            ));
            index
        });
        // The same URI/address now has a custom ref meaning. A later query
        // must not retain the previously confirmed builtin reference.
        let changed = format!(r"\renewcommand\ref[1]{{#1}}{source}");
        document(&mut workspace, &uri, &changed);
        with_cache(|| {
            let index = named_references::Index::new(&workspace, &uri);
            assert!(!Arc::ptr_eq(&retained, &index));
            assert!(index.target(&uri, changed.rfind("key").unwrap()).is_none());
        });
    }

    #[test]
    fn label_proofs_distinguish_exact_project_scopes_and_workspaces() {
        let consumer = Url::parse("untitled:consumer.tex").unwrap();
        let provider = Url::parse("untitled:provider.tex").unwrap();
        let mut workspace = Workspace::default();
        document(&mut workspace, &consumer, r"\typeout{\label{ghost}}");
        document(&mut workspace, &provider, r"\renewcommand\typeout[1]{#1}");
        let mut other = workspace.clone();
        document(&mut other, &provider, "No redefinition");
        let single = BTreeSet::from([consumer.clone()]);
        let both = BTreeSet::from([consumer.clone(), provider]);
        let entry = &workspace.documents[&consumer].syntax.labels[0];
        with_cache(|| {
            let local = label_declarations::Index::for_scope(&workspace, &single);
            assert!(!local.contains(&consumer, entry));
            assert!(Arc::ptr_eq(
                &local,
                &label_declarations::Index::for_scope(&workspace, &single)
            ));
            assert!(
                label_declarations::Index::for_scope(&workspace, &both).contains(&consumer, entry)
            );
            assert!(!label_declarations::Index::for_scope(&other, &both).contains(&consumer, entry));
            assert!(!label_declarations::Index::for_scope(&workspace, &single)
                .contains(&consumer, entry));
        });
    }

    #[test]
    fn full_caches_compute_results_without_retaining_more_analyses() {
        with_cache(|| {
            let get = |key, bytes| {
                shared(
                    (0, Url::parse(&format!("untitled:{key}")).unwrap()),
                    |cache| &mut cache.named,
                    named_references::Index::default,
                    |_| bytes,
                )
            };
            let oversized = get("oversized", MAX_BYTES + 1);
            assert!(!Arc::ptr_eq(&oversized, &get("oversized", MAX_BYTES + 1)));
            let retained = get("retained", MAX_BYTES);
            assert!(Arc::ptr_eq(&retained, &get("retained", MAX_BYTES)));
            let uncached = get("no-space", 1);
            assert!(!Arc::ptr_eq(&uncached, &get("no-space", 1)));
        });
        with_cache(|| {
            let workspace = Workspace::default();
            for index in 0..MAX_ENTRIES {
                named_references::Index::new(
                    &workspace,
                    &Url::parse(&format!("untitled:{index}")).unwrap(),
                );
            }
            let overflow = Url::parse("untitled:overflow").unwrap();
            let first = named_references::Index::new(&workspace, &overflow);
            assert!(!Arc::ptr_eq(
                &first,
                &named_references::Index::new(&workspace, &overflow)
            ));
        });
    }
}
