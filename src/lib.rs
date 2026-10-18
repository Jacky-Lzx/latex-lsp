pub mod build;
pub mod build_log;
pub mod catalog;
pub mod config;
pub mod dispatch;
pub mod editing;
pub mod features;
pub mod file_operations;
pub mod navigation;
pub mod path_completion;
pub mod recorder;
pub mod synctex;
pub mod syntax;
pub mod text;
pub mod workspace;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;
use tower_lsp::{jsonrpc::Result, lsp_types::*, Client, LanguageServer};
use workspace::{Document, Workspace};

fn file_operation_options() -> FileOperationRegistrationOptions {
    FileOperationRegistrationOptions {
        filters: vec![FileOperationFilter {
            scheme: Some("file".into()),
            pattern: FileOperationPattern {
                glob: "**/*".into(),
                matches: None,
                options: None,
            },
        }],
    }
}

pub struct Backend {
    client: Client,
    workspace: Arc<RwLock<Workspace>>,
    builds: build::BuildManager,
    sync_queries: synctex::Queries,
    shutdown_requested: AtomicBool,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let workspace = Arc::new(RwLock::new(Workspace::default()));
        Self {
            builds: build::BuildManager::new(client.clone(), workspace.clone()),
            sync_queries: synctex::Queries::default(),
            client,
            workspace,
            shutdown_requested: AtomicBool::new(false),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
    }

    pub async fn cancel_progress(&self, params: WorkDoneProgressCancelParams) {
        self.builds.cancel_progress(&params.token);
    }

    async fn publish(&self, uri: Url) {
        let workspace = self.workspace.read().await;
        let snapshot = workspace
            .documents
            .get(&uri)
            .filter(|doc| doc.version.is_some())
            .map(|doc| (workspace.diagnostics(&uri), doc.version));
        drop(workspace);
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
        workspace.build_progress = params
            .capabilities
            .window
            .as_ref()
            .and_then(|window| window.work_done_progress)
            .unwrap_or(false);
        if let Some(options) = params.initialization_options {
            workspace.config = config::Config::parse(options)
                .map_err(tower_lsp::jsonrpc::Error::invalid_params)?;
        }
        workspace.versioned_edits = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false);
        workspace.apply_edits = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.apply_edit)
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
        workspace.code_action_literals = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.code_action.as_ref())
            .is_some_and(|action| action.code_action_literal_support.is_some());
        workspace.folding_limit = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.folding_range.as_ref())
            .and_then(|folding| folding.range_limit);
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
                    trigger_characters: Some(vec!["\\".into(), "{".into(), ",".into(), "/".into()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        resolve_provider: Some(false),
                        ..Default::default()
                    },
                )),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "latex.build".into(),
                        "latex.cancelBuild".into(),
                        "latex.buildStatus".into(),
                        "latex.forwardSearch".into(),
                        "latex.inverseSearch".into(),
                        "latex.applyQuickFix".into(),
                    ],
                    ..Default::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                        will_rename: Some(file_operation_options()),
                        did_rename: Some(file_operation_options()),
                        ..Default::default()
                    }),
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
        self.sync_queries.shutdown().await;
        self.builds.shutdown().await;
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
        workspace.clear_build_diagnostics(&uri);
        workspace.load_dependencies(&uri, 0);
        drop(workspace);
        self.publish(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        self.publish(uri.clone()).await;
        let on_save = self.workspace.read().await.config.build.on_save;
        if on_save {
            if let Err(error) = self.builds.on_save(uri).await {
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
            }
        }
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
            workspace.clear_build_diagnostics(&change.uri);
            workspace.load_dependencies(&change.uri, 0);
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        match config::Config::parse(params.settings) {
            Ok(config) => {
                self.workspace.write().await.config = config;
            }
            Err(error) => self.client.log_message(MessageType::WARNING, error).await,
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        let argument = params.arguments.first();
        if params.command == "latex.cancelBuild" && argument.is_none() {
            return Ok(Some(
                serde_json::json!({"cancelled":self.builds.cancel(None)}),
            ));
        }
        let uri = argument
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("uri").and_then(serde_json::Value::as_str))
            })
            .and_then(|value| Url::parse(value).ok())
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(
                    "Expected a document URI as the first command argument",
                )
            })?;
        match params.command.as_str() {
            "latex.applyQuickFix" => {
                let request: editing::FixRequest =
                    serde_json::from_value(argument.cloned().unwrap_or_default()).map_err(
                        |_| tower_lsp::jsonrpc::Error::invalid_params("Invalid quick fix request"),
                    )?;
                let edit = editing::resolve_fix(&*self.workspace.read().await, request)
                    .ok_or_else(|| {
                        tower_lsp::jsonrpc::Error::invalid_params(
                            "Quick fix is outdated; request code actions again",
                        )
                    })?;
                let response = self.client.apply_edit(edit).await?;
                Ok(Some(serde_json::to_value(response).unwrap()))
            }
            "latex.build" => self.builds.start(uri).await.map(Some),
            "latex.forwardSearch" | "latex.inverseSearch" => {
                let project = synctex::prepare(&*self.workspace.read().await, &uri)?;
                let available = |root: &Url| {
                    let status = self.builds.status(root);
                    status["status"] == "idle"
                        || (status["status"] == "success" && status["stale"] != true)
                };
                if !available(&project.root) {
                    return Err(tower_lsp::jsonrpc::Error::invalid_params(
                        "Finish a successful build before querying SyncTeX",
                    ));
                }
                let value = argument.cloned().unwrap_or_default();
                let inverse = params.command == "latex.inverseSearch";
                let mut task = self.sync_queries.start(project.clone(), value, inverse)?;
                dispatch::release();
                let result = task.wait().await?;
                let current = synctex::prepare(&*self.workspace.read().await, &uri)?;
                if current != project || !available(&current.root) {
                    return Err(tower_lsp::jsonrpc::Error::invalid_params(
                        "Build artifacts changed during the SyncTeX query; retry after compiling",
                    ));
                }
                Ok(Some(result))
            }
            "latex.cancelBuild" | "latex.buildStatus" => {
                let root = self
                    .workspace
                    .read()
                    .await
                    .root_document(&uri)
                    .map_err(tower_lsp::jsonrpc::Error::invalid_params)?;
                if params.command == "latex.cancelBuild" {
                    Ok(Some(
                        serde_json::json!({"cancelled":self.builds.cancel(Some(&root))}),
                    ))
                } else {
                    Ok(Some(self.builds.status(&root)))
                }
            }
            _ => Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
                "Unknown command: {}",
                params.command
            ))),
        }
    }

    async fn will_rename_files(&self, params: RenameFilesParams) -> Result<Option<WorkspaceEdit>> {
        file_operations::will_rename(&*self.workspace.read().await, &params.files)
    }

    async fn did_rename_files(&self, params: RenameFilesParams) {
        match file_operations::Renames::new(&params.files) {
            Ok(renames) => self.builds.files_renamed(&renames).await,
            Err(error) => {
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
                return;
            }
        }
        let mut workspace = self.workspace.write().await;
        match file_operations::did_rename(&mut workspace, &params.files) {
            Ok(affected) => {
                let diagnostics = affected
                    .into_iter()
                    .map(|uri| {
                        let version = workspace.documents.get(&uri).and_then(|doc| doc.version);
                        let diagnostics = workspace.diagnostics(&uri);
                        (uri, diagnostics, version)
                    })
                    .collect::<Vec<_>>();
                drop(workspace);
                for (uri, diagnostics, version) in diagnostics {
                    self.client
                        .publish_diagnostics(uri, diagnostics, version)
                        .await;
                }
            }
            Err(error) => {
                drop(workspace);
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
            }
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

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let params = params.text_document_position_params;
        Ok(Some(editing::highlights(
            &*self.workspace.read().await,
            &params.text_document.uri,
            params.position,
        )))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace.documents.get(&params.text_document.uri) else {
            return Ok(None);
        };
        editing::selection_ranges(doc, &params.positions)
            .map(Some)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Invalid selection position"))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        Ok(Some(editing::code_actions(
            &*self.workspace.read().await,
            &params.text_document.uri,
            params.range,
            &params.context,
        )))
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

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        Ok(Some(navigation::workspace_symbols(
            &*self.workspace.read().await,
            &params.query,
        )))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let workspace = self.workspace.read().await;
        Ok(workspace
            .documents
            .get(&params.text_document.uri)
            .map(|doc| navigation::folding_ranges(doc, workspace.folding_limit)))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        Ok(Some(navigation::document_links(
            &*self.workspace.read().await,
            &params.text_document.uri,
        )))
    }
}
