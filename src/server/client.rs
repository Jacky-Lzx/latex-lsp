//! Outbound messages and cancellation-safe server-to-client requests.
use crate::rpc::{Error, Result};
use lsp_types::{notification, request, *};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Id {
    Number(i64),
    String(String),
    Null,
}

struct Inner {
    outgoing: mpsc::Sender<Value>,
    next_id: AtomicU64,
    closed: AtomicBool,
    pending: Mutex<HashMap<Id, oneshot::Sender<Result<Value>>>>,
}

#[derive(Clone)]
pub struct Client(Arc<Inner>);

/// Removing the waiter on drop also covers timeout and parent cancellation.
struct Waiting {
    client: Client,
    id: Id,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.client.0.pending.lock().unwrap().remove(&self.id);
    }
}

impl Client {
    pub fn new() -> (Self, mpsc::Receiver<Value>) {
        let (outgoing, receiver) = mpsc::channel(64);
        (
            Self(Arc::new(Inner {
                outgoing,
                next_id: AtomicU64::new(0),
                closed: AtomicBool::new(false),
                pending: Mutex::default(),
            })),
            receiver,
        )
    }

    pub(crate) async fn send(&self, value: Value) -> Result<()> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Err(Error::request_cancelled());
        }
        self.0
            .outgoing
            .send(value)
            .await
            .map_err(|_| Error::request_cancelled())
    }

    pub(crate) fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        self.0.pending.lock().unwrap().clear();
    }

    /// Unknown, duplicate and late responses are harmless. No handler is run
    /// while holding the pending-request lock or the source dispatch turn.
    pub(crate) fn respond(&self, id: &Id, response: Result<Value>) {
        let sender = self.0.pending.lock().unwrap().remove(id);
        if let Some(sender) = sender {
            let _ = sender.send(response);
        }
    }

    pub(crate) async fn send_request<R: request::Request>(
        &self,
        params: R::Params,
    ) -> Result<R::Result> {
        let value = serde_json::to_value(params).map_err(|_| Error::internal_error())?;
        let id = Id::Number(self.0.next_id.fetch_add(1, Ordering::Relaxed) as i64);
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.0.pending.lock().unwrap();
            if self.0.closed.load(Ordering::SeqCst) {
                return Err(Error::request_cancelled());
            }
            pending.insert(id.clone(), sender);
        }
        let _waiting = Waiting {
            client: self.clone(),
            id: id.clone(),
        };
        self.send(json!({"jsonrpc":"2.0","id":id,"method":R::METHOD,"params":value}))
            .await?;
        let result = receiver.await.map_err(|_| Error::request_cancelled())??;
        serde_json::from_value(result).map_err(|_| Error::internal_error())
    }

    pub(crate) async fn send_notification<N: notification::Notification>(&self, params: N::Params) {
        if let Ok(params) = serde_json::to_value(params) {
            let _ = self
                .send(json!({"jsonrpc":"2.0","method":N::METHOD,"params":params}))
                .await;
        }
    }

    pub(crate) async fn log_message(&self, typ: MessageType, message: impl ToString) {
        self.send_notification::<notification::LogMessage>(LogMessageParams {
            typ,
            message: message.to_string(),
        })
        .await;
    }

    pub(crate) async fn publish_diagnostics(
        &self,
        uri: Url,
        diagnostics: Vec<Diagnostic>,
        version: Option<i32>,
    ) {
        self.send_notification::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        })
        .await;
    }

    pub(crate) async fn register_capability(&self, registrations: Vec<Registration>) -> Result<()> {
        self.send_request::<request::RegisterCapability>(RegistrationParams { registrations })
            .await
    }

    pub(crate) async fn apply_edit(
        &self,
        edit: WorkspaceEdit,
    ) -> Result<ApplyWorkspaceEditResponse> {
        self.send_request::<request::ApplyWorkspaceEdit>(ApplyWorkspaceEditParams {
            label: None,
            edit,
        })
        .await
    }

    pub(crate) async fn workspace_diagnostic_refresh(&self) -> Result<()> {
        self.send_request::<request::WorkspaceDiagnosticRefresh>(())
            .await
    }

    pub(crate) async fn inlay_hint_refresh(&self) -> Result<()> {
        self.send_request::<request::InlayHintRefreshRequest>(())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_removes_waiters_and_late_or_duplicate_responses_are_ignored() {
        let (client, mut outgoing) = Client::new();
        let worker = client.clone();
        let cancelled = tokio::spawn(async move { worker.workspace_diagnostic_refresh().await });
        let first = outgoing.recv().await.unwrap();
        let id: Id = serde_json::from_value(first["id"].clone()).unwrap();
        assert_eq!(client.0.pending.lock().unwrap().len(), 1);
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert!(client.0.pending.lock().unwrap().is_empty());
        client.respond(&id, Ok(Value::Null));
        client.respond(&id, Err(Error::internal_error()));

        let worker = client.clone();
        let next = tokio::spawn(async move { worker.workspace_diagnostic_refresh().await });
        let second = outgoing.recv().await.unwrap();
        let next_id: Id = serde_json::from_value(second["id"].clone()).unwrap();
        assert_ne!(id, next_id);
        client.respond(&id, Ok(Value::Null));
        assert!(!next.is_finished());
        client.respond(&next_id, Ok(Value::Null));
        next.await.unwrap().unwrap();
        assert!(client.0.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn disconnect_wakes_pending_requests_and_rejects_new_requests() {
        let (client, mut outgoing) = Client::new();
        let worker = client.clone();
        let pending = tokio::spawn(async move { worker.workspace_diagnostic_refresh().await });
        outgoing.recv().await.unwrap();
        client.close();
        assert_eq!(
            pending.await.unwrap().unwrap_err().code,
            crate::rpc::ErrorCode::RequestCancelled
        );
        assert_eq!(
            client
                .workspace_diagnostic_refresh()
                .await
                .unwrap_err()
                .code,
            crate::rpc::ErrorCode::RequestCancelled
        );
        assert!(client.0.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn client_errors_are_returned_without_affecting_later_requests() {
        let (client, mut outgoing) = Client::new();
        let worker = client.clone();
        let pending = tokio::spawn(async move { worker.workspace_diagnostic_refresh().await });
        let message = outgoing.recv().await.unwrap();
        let id = serde_json::from_value(message["id"].clone()).unwrap();
        let error = Error::invalid_params("unsupported refresh");
        client.respond(&id, Err(error.clone()));
        assert_eq!(pending.await.unwrap(), Err(error));
        assert!(client.0.pending.lock().unwrap().is_empty());
    }
}
