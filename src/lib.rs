pub mod catalog;
pub mod features;
pub mod syntax;
pub mod text;
pub mod workspace;

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tower_lsp::{jsonrpc::Result, lsp_types::*, Client, LanguageServer};
use workspace::{Document, Workspace};

pub struct Backend {
    client: Client,
    workspace: RwLock<Workspace>,
    shutdown_requested: AtomicBool,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: RwLock::new(Workspace::default()),
            shutdown_requested: AtomicBool::new(false),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
    }

    async fn publish(&self, uri: Url) {
        let snapshot = self
            .workspace
            .read()
            .await
            .documents
            .get(&uri)
            .filter(|doc| doc.version.is_some())
            .map(|doc| (features::diagnostics(doc), doc.version));
        if let Some((diagnostics, version)) = snapshot {
            self.client
                .publish_diagnostics(uri, diagnostics, version)
                .await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut workspace = self.workspace.write().await;
        workspace.versioned_edits = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false);
        workspace.watch_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|watch| watch.dynamic_registration)
            .unwrap_or(false);
        workspace.snippets = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.completion.as_ref())
            .and_then(|completion| completion.completion_item.as_ref())
            .and_then(|item| item.snippet_support)
            .unwrap_or(false);
        workspace.hierarchical_symbols = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.document_symbol.as_ref())
            .and_then(|symbols| symbols.hierarchical_document_symbol_support)
            .unwrap_or(false);
        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                if let Ok(root) = folder.uri.to_file_path() {
                    workspace.add_root(root);
                }
            }
        }
        if workspace.roots.is_empty() {
            if let Some(uri) = params.root_uri {
                if let Ok(root) = uri.to_file_path() {
                    workspace.add_root(root);
                }
            }
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..Default::default()
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["\\".into(), "{".into(), ",".into()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "latex-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        if self.workspace.read().await.watch_files {
            let registration = Registration {
                id: "latex-lsp-watch-files".into(),
                method: "workspace/didChangeWatchedFiles".into(),
                register_options: Some(serde_json::json!({
                    "watchers": [{"globPattern": "**/*.{tex,bib,sty,cls}", "kind": 7}]
                })),
            };
            if let Err(error) = self.client.register_capability(vec![registration]).await {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("File watcher registration failed: {error}"),
                    )
                    .await;
            }
        }
        self.client
            .log_message(MessageType::INFO, "latex-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        self.workspace
            .write()
            .await
            .open(item.uri.clone(), item.text, item.version);
        self.publish(item.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut workspace = self.workspace.write().await;
        let Some(doc) = workspace.documents.get(&uri) else {
            return;
        };
        if !doc
            .version
            .is_some_and(|version| params.text_document.version > version)
        {
            return;
        }
        let Some(text) = doc.text.changed(params.content_changes) else {
            drop(workspace);
            self.client
                .log_message(
                    MessageType::WARNING,
                    "Ignored invalid edit range; document content was preserved",
                )
                .await;
            return;
        };
        workspace.documents.insert(
            uri.clone(),
            Document::new(&uri, text.source, Some(params.text_document.version)),
        );
        workspace.load_dependencies(&uri, 0);
        drop(workspace);
        self.publish(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.publish(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut workspace = self.workspace.write().await;
        workspace.documents.remove(&uri);
        workspace.reload(&uri);
        drop(workspace);
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let mut workspace = self.workspace.write().await;
        for change in params.changes {
            workspace.reload(&change.uri);
            workspace.load_dependencies(&change.uri, 0);
        }
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        let mut workspace = self.workspace.write().await;
        for folder in params.event.removed {
            if let Ok(root) = folder.uri.to_file_path() {
                workspace.roots.remove(&root);
            }
        }
        let roots = workspace.roots.clone();
        workspace.documents.retain(|uri, doc| {
            doc.version.is_some()
                || uri
                    .to_file_path()
                    .ok()
                    .is_some_and(|path| roots.iter().any(|root| path.starts_with(root)))
        });
        for folder in params.event.added {
            if let Ok(root) = folder.uri.to_file_path() {
                workspace.add_root(root);
            }
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        Ok(Some(CompletionResponse::Array(features::completion(
            &*self.workspace.read().await,
            &position.text_document.uri,
            position.position,
        ))))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params;
        Ok(features::hover(
            &*self.workspace.read().await,
            &position.text_document.uri,
            position.position,
        ))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let position = params.text_document_position_params;
        let locations = features::definitions(
            &*self.workspace.read().await,
            &position.text_document.uri,
            position.position,
        );
        Ok(Some(GotoDefinitionResponse::Array(locations)))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let position = params.text_document_position;
        Ok(Some(features::references(
            &*self.workspace.read().await,
            &position.text_document.uri,
            position.position,
            params.context.include_declaration,
        )))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        features::prepare_rename(
            &*self.workspace.read().await,
            &params.text_document.uri,
            params.position,
        )
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let position = params.text_document_position;
        features::rename(
            &*self.workspace.read().await,
            &position.text_document.uri,
            position.position,
            &params.new_name,
        )
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let workspace = self.workspace.read().await;
        let uri = params.text_document.uri;
        Ok(workspace.documents.get(&uri).map(|doc| {
            if workspace.hierarchical_symbols {
                DocumentSymbolResponse::Nested(features::symbols(doc))
            } else {
                DocumentSymbolResponse::Flat(features::flat_symbols(doc, &uri))
            }
        }))
    }
}
