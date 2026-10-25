//! Pull reports and capability-aware delivery share the workspace's diagnostics.
use crate::server::client::Client;
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::project::workspace::Workspace;

pub(crate) type Snapshot = (Url, Vec<Diagnostic>, Option<i32>);
pub(crate) const IDENTIFIER: &str = "latex-lsp";

#[derive(Default)]
struct Refresh {
    running: bool,
    dirty: bool,
    closed: bool,
}

#[derive(Default)]
struct Push {
    revision: u64,
    running: bool,
    closed: bool,
    pending: BTreeSet<Url>,
}

/// A connection's negotiated diagnostic delivery mode.
#[derive(Clone, Default)]
pub struct Delivery {
    pull: bool,
    refresh: Option<Arc<Mutex<Refresh>>>,
    push: Arc<Mutex<Push>>,
}

impl Delivery {
    pub(crate) fn new(pull: bool, refresh: bool) -> Self {
        Self {
            pull,
            refresh: (pull && refresh).then(Arc::default),
            push: Arc::default(),
        }
    }

    pub(crate) fn is_pull(&self) -> bool {
        self.pull
    }

    pub(crate) fn close(&self) {
        self.push.lock().unwrap().closed = true;
        if let Some(refresh) = &self.refresh {
            refresh.lock().unwrap().closed = true;
        }
    }

    /// Called under the workspace write lock, before changing its inputs.
    pub(crate) fn invalidate(&self) {
        self.push.lock().unwrap().revision += 1;
    }

    /// A single worker coalesces edits and never holds a workspace lock while
    /// analysing a snapshot or waiting for the client to receive diagnostics.
    pub(crate) fn schedule(
        &self,
        client: &Client,
        workspace: Arc<tokio::sync::RwLock<Workspace>>,
        affected: BTreeSet<Url>,
    ) {
        let mut push = self.push.lock().unwrap();
        if push.closed {
            return;
        }
        push.pending.extend(affected);
        if push.running {
            return;
        }
        push.running = true;
        drop(push);
        let state = self.push.clone();
        let client = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(75)).await;
                let (snapshot, uris, revision) = {
                    let workspace = workspace.read().await;
                    let push = state.lock().unwrap();
                    if push.closed {
                        return;
                    }
                    let uris: BTreeSet<_> =
                        push.pending
                            .iter()
                            .cloned()
                            .chain(workspace.documents.iter().filter_map(|(uri, doc)| {
                                doc.version.is_some().then_some(uri.clone())
                            }))
                            .collect();
                    (workspace.clone(), uris, push.revision)
                };
                let cancellation = state.clone();
                let notifications = tokio::task::spawn_blocking(move || {
                    snapshot.diagnostic_snapshots_cancelled(uris, || {
                        let push = cancellation.lock().unwrap();
                        push.closed || push.revision != revision
                    })
                })
                .await;
                {
                    let mut push = state.lock().unwrap();
                    if push.closed {
                        return;
                    }
                    if push.revision != revision {
                        continue;
                    }
                    push.pending.clear();
                }
                if let Ok(notifications) = notifications {
                    for (uri, diagnostics, version) in notifications {
                        {
                            let push = state.lock().unwrap();
                            if push.closed || push.revision != revision {
                                break;
                            }
                        }
                        client.publish_diagnostics(uri, diagnostics, version).await;
                    }
                }
                let mut push = state.lock().unwrap();
                if push.closed || push.revision == revision && push.pending.is_empty() {
                    push.running = false;
                    return;
                }
            }
        });
    }

    pub(crate) async fn publish(&self, client: &Client, snapshots: Vec<Snapshot>) {
        if !self.pull {
            for (uri, diagnostics, version) in snapshots {
                client.publish_diagnostics(uri, diagnostics, version).await;
            }
            return;
        }
        let Some(refresh) = &self.refresh else {
            return;
        };
        {
            let mut state = refresh.lock().unwrap();
            if state.closed {
                return;
            }
            state.dirty = true;
            if state.running {
                return;
            }
            state.running = true;
        }
        let refresh = refresh.clone();
        let client = client.clone();
        // A client may pull before acknowledging refresh. Never hold a request
        // queue turn or a workspace lock while waiting for that acknowledgement.
        tokio::spawn(async move {
            loop {
                // Nearby edits share one workspace-wide recalculation.
                tokio::time::sleep(Duration::from_millis(75)).await;
                {
                    let mut state = refresh.lock().unwrap();
                    if state.closed {
                        state.running = false;
                        return;
                    }
                    state.dirty = false;
                }
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    client.workspace_diagnostic_refresh(),
                )
                .await;
                let mut state = refresh.lock().unwrap();
                // Coalesce changes while the client handles one refresh, but
                // retain a later invalidation if it pulled an earlier snapshot.
                if state.closed || !matches!(result, Ok(Ok(()))) || !state.dirty {
                    state.running = false;
                    return;
                }
            }
        });
    }
}

const MAX_DOCUMENTS: usize = 128;
const MAX_BYTES: usize = 2 * 1024 * 1024;

struct Cached {
    items: Vec<Diagnostic>,
    version: Option<i32>,
    result_id: String,
    sequence: u64,
    bytes: usize,
}

/// Result IDs are scoped to the URI, version and complete diagnostic payload.
/// Oversized or evicted reports safely fall back to full responses.
#[derive(Default)]
pub(crate) struct Cache {
    snapshots: BTreeMap<Url, Cached>,
    sequence: u64,
    bytes: usize,
}

impl Cache {
    pub(crate) fn remove(&mut self, uri: &Url) {
        if let Some(snapshot) = self.snapshots.remove(uri) {
            self.bytes -= snapshot.bytes;
        }
    }

    pub(crate) fn report(
        &mut self,
        (uri, items, version): Snapshot,
        previous: Option<&str>,
    ) -> DocumentDiagnosticReportKind {
        self.sequence += 1;
        if let Some(snapshot) = self.snapshots.get_mut(&uri) {
            if snapshot.version == version && snapshot.items == items {
                snapshot.sequence = self.sequence;
                return if previous == Some(snapshot.result_id.as_str()) {
                    DocumentDiagnosticReportKind::Unchanged(UnchangedDocumentDiagnosticReport {
                        result_id: snapshot.result_id.clone(),
                    })
                } else {
                    DocumentDiagnosticReportKind::Full(FullDocumentDiagnosticReport {
                        result_id: Some(snapshot.result_id.clone()),
                        items,
                    })
                };
            }
        }
        self.remove(&uri);
        let bytes = uri.as_str().len() + serde_json::to_vec(&items).unwrap().len();
        if bytes > MAX_BYTES {
            return FullDocumentDiagnosticReport {
                result_id: None,
                items,
            }
            .into();
        }
        while self.snapshots.len() >= MAX_DOCUMENTS || self.bytes + bytes > MAX_BYTES {
            let Some(oldest) = self
                .snapshots
                .iter()
                .min_by_key(|(_, snapshot)| snapshot.sequence)
                .map(|(uri, _)| uri.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        let result_id = format!("diagnostic-{}", self.sequence);
        self.bytes += bytes;
        self.snapshots.insert(
            uri,
            Cached {
                items: items.clone(),
                version,
                result_id: result_id.clone(),
                sequence: self.sequence,
                bytes,
            },
        );
        FullDocumentDiagnosticReport {
            result_id: Some(result_id),
            items,
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(id: usize, message: &str, version: Option<i32>) -> Snapshot {
        (
            Url::parse(&format!("file:///diagnostic-cache/{id}.tex")).unwrap(),
            vec![Diagnostic::new_simple(Range::default(), message.into())],
            version,
        )
    }

    fn full_id(report: DocumentDiagnosticReportKind) -> String {
        match report {
            DocumentDiagnosticReportKind::Full(report) => report.result_id.unwrap(),
            _ => panic!("expected full report"),
        }
    }

    #[test]
    fn result_ids_require_matching_uri_version_and_payload_and_survive_empty_reports() {
        let mut cache = Cache::default();
        let first = snapshot(1, "first", Some(1));
        let id = full_id(cache.report(first.clone(), None));
        assert!(matches!(
            cache.report(first.clone(), Some(&id)),
            DocumentDiagnosticReportKind::Unchanged(_)
        ));
        assert_eq!(full_id(cache.report(first.clone(), Some("unknown"))), id);
        assert_ne!(
            full_id(cache.report(snapshot(2, "first", Some(1)), Some(&id))),
            id
        );
        let newer = full_id(cache.report(snapshot(1, "first", Some(2)), Some(&id)));
        assert_ne!(newer, id);
        let changed = full_id(cache.report(snapshot(1, "second", Some(2)), Some(&newer)));
        assert_ne!(changed, newer);
        let empty = (first.0.clone(), vec![], Some(2));
        let cleared = full_id(cache.report(empty.clone(), Some(&changed)));
        assert!(matches!(
            cache.report(empty, Some(&cleared)),
            DocumentDiagnosticReportKind::Unchanged(_)
        ));
        cache.remove(&first.0);
        assert_ne!(full_id(cache.report(first, Some(&id))), id);
    }

    #[test]
    fn bounded_cache_evicts_old_reports_and_does_not_retain_oversized_payloads() {
        let mut cache = Cache::default();
        let first = full_id(cache.report(snapshot(0, "first", None), None));
        for id in 1..=MAX_DOCUMENTS {
            cache.report(snapshot(id, "next", None), None);
        }
        assert_ne!(
            full_id(cache.report(snapshot(0, "first", None), Some(&first))),
            first
        );
        let large = snapshot(MAX_DOCUMENTS + 1, &"x".repeat(MAX_BYTES), None);
        let report = cache.report(large.clone(), None);
        assert!(matches!(
            report,
            DocumentDiagnosticReportKind::Full(FullDocumentDiagnosticReport {
                result_id: None,
                ..
            })
        ));
        assert!(!cache.snapshots.contains_key(&large.0));
        assert!(cache.bytes <= MAX_BYTES);
        assert!(cache.snapshots.len() <= MAX_DOCUMENTS);
        let mut cache = Cache::default();
        let half = "x".repeat(MAX_BYTES / 2);
        let old = full_id(cache.report(snapshot(1, &half, None), None));
        cache.report(snapshot(2, &half, None), None);
        assert!(!cache.snapshots.contains_key(&snapshot(1, "", None).0));
        assert_ne!(
            full_id(cache.report(snapshot(1, &half, None), Some(&old))),
            old
        );
        assert!(cache.bytes <= MAX_BYTES);
    }
}
