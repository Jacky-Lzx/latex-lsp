//! Runtime workspace mutation has one revision boundary for every consumer.
use super::workspace::Workspace;
use std::ops::{Deref, DerefMut};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{OwnedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct WorkspaceUpdate<'a> {
    workspace: RwLockWriteGuard<'a, Workspace>,
    revision: &'a AtomicU64,
    changed: bool,
}

impl Deref for WorkspaceUpdate<'_> {
    type Target = Workspace;
    fn deref(&self) -> &Workspace {
        &self.workspace
    }
}

impl DerefMut for WorkspaceUpdate<'_> {
    fn deref_mut(&mut self) -> &mut Workspace {
        if !self.changed {
            self.revision.fetch_add(1, Ordering::SeqCst);
            self.changed = true;
        }
        &mut self.workspace
    }
}

#[derive(Clone, Default)]
pub struct WorkspaceStore {
    workspace: Arc<RwLock<Workspace>>,
    revision: Arc<AtomicU64>,
}

impl WorkspaceStore {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace: Arc::new(RwLock::new(workspace)),
            revision: Arc::default(),
        }
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, Workspace> {
        self.workspace.read().await
    }

    pub async fn read_owned(self) -> OwnedRwLockReadGuard<Workspace> {
        self.workspace.read_owned().await
    }

    /// Invalidate old analyses on the first mutable access, including when
    /// reindexing replaces the entire Workspace value.
    pub async fn write(&self) -> WorkspaceUpdate<'_> {
        let workspace = self.workspace.write().await;
        WorkspaceUpdate {
            workspace,
            revision: &self.revision,
            changed: false,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_invalidate_snapshots_even_when_replacing_the_workspace() {
        let store = WorkspaceStore::default();
        let revision = store.revision();
        let snapshot = store.read().await.clone();
        assert_eq!(store.revision(), revision);
        assert!(!store.write().await.client.snippets);
        assert_eq!(
            store.revision(),
            revision,
            "Reading through a write guard is not an input change"
        );
        {
            let mut workspace = store.write().await;
            workspace.client.snippets = true;
        }
        assert!(store.revision() > revision);
        assert!(!snapshot.client.snippets);
        let revision = store.revision();
        *store.write().await = Workspace::default();
        assert!(store.revision() > revision);
        assert!(!store.read().await.client.snippets);
    }
}
