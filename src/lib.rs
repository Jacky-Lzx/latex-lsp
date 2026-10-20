mod anchors;
pub mod build;
pub mod build_log;
pub mod catalog;
mod code_inputs;
pub mod code_lens;
pub mod colors;
mod conditionals;
pub mod config;
mod counters;
pub mod diagnostic;
pub mod dispatch;
pub mod editing;
mod fancyvrb;
pub mod features;
mod file_checks;
mod file_fixes;
pub mod file_operations;
mod generated;
pub mod graphics;
mod image_fixes;
mod listings;
mod named_commands;
pub mod navigation;
mod output;
pub mod path_completion;
mod primitives;
mod project_diagnostics;
pub mod recorder;
mod saved_text;
pub mod semantic;
mod semantic_cache;
pub mod signature;
mod spelling_fixes;
pub mod synctex;
pub mod syntax;
pub mod text;
pub mod workspace;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{Mutex, RwLock};
use tower_lsp::{jsonrpc::Result, lsp_types::*, Client, LanguageServer};
use workspace::{document_uri, Document, Workspace};

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

fn validate_diagnostic_identifier(identifier: Option<&str>) -> Result<()> {
    if identifier.is_some_and(|identifier| identifier != diagnostic::IDENTIFIER) {
        Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Unknown diagnostic provider",
        ))
    } else {
        Ok(())
    }
}

pub struct Backend {
    client: Client,
    workspace: Arc<RwLock<Workspace>>,
    builds: build::BuildManager,
    sync_queries: synctex::Queries,
    shutdown_requested: AtomicBool,
    semantic_cache: Mutex<semantic_cache::Cache>,
    diagnostic_cache: Mutex<diagnostic::Cache>,
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
            semantic_cache: Mutex::new(semantic_cache::Cache::default()),
            diagnostic_cache: Mutex::new(diagnostic::Cache::default()),
        }
    }

    /// Cancel and reap external work, retaining final notifications for shutdown.
    pub fn cleanup(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let builds = self.builds.clone();
        let queries = self.sync_queries.clone();
        let workspace = self.workspace.clone();
        async move {
            queries.shutdown().await;
            builds.shutdown().await;
            workspace.read().await.diagnostic_delivery.close();
        }
    }

    /// Stop outbound build messages before reaping work on a closed transport.
    pub fn disconnect(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let builds = self.builds.clone();
        let cleanup = self.cleanup();
        async move {
            builds.disconnect();
            cleanup.await;
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
    }

    pub async fn cancel_progress(&self, params: WorkDoneProgressCancelParams) {
        self.builds.cancel_progress(&params.token);
    }

    async fn publish_open(&self) {
        self.publish_affected(Default::default()).await;
    }

    async fn publish_affected(&self, affected: std::collections::BTreeSet<Url>) {
        let workspace = self.workspace.read().await;
        let delivery = workspace.diagnostic_delivery.clone();
        let snapshots = if delivery.is_pull() {
            vec![]
        } else {
            let uris = affected.into_iter().chain(
                workspace
                    .documents
                    .iter()
                    .filter(|(_, doc)| doc.version.is_some())
                    .map(|(uri, _)| uri.clone()),
            );
            workspace.diagnostic_snapshots(uris)
        };
        drop(workspace);
        delivery.publish(&self.client, snapshots).await;
    }

    async fn refresh_file_events(
        &self,
        changes: Result<file_operations::FileChanges>,
        from_watcher: bool,
    ) {
        let changes = match changes {
            Ok(changes) => changes,
            Err(error) => {
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
                return;
            }
        };
        if from_watcher {
            self.builds.watched_files_changed(&changes).await;
        } else {
            self.builds.files_changed(&changes).await;
        }
        let affected = {
            let mut workspace = self.workspace.write().await;
            if from_watcher {
                changes.refresh_watched(&mut workspace)
            } else {
                changes.refresh(&mut workspace)
            }
        };
        {
            let mut cache = self.semantic_cache.lock().await;
            for uri in &affected {
                cache.remove(uri);
            }
        }
        if !from_watcher {
            let mut cache = self.diagnostic_cache.lock().await;
            for uri in &affected {
                cache.remove(uri);
            }
        }
        // Pull requests recalculate the payload before consulting the cache.
        // Keep result IDs when a delayed watcher changes neither it nor the version.
        self.publish_affected(affected).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut workspace = self.workspace.write().await;
        let pull_diagnostics = params
            .capabilities
            .text_document
            .as_ref()
            .is_some_and(|document| document.diagnostic.is_some());
        let diagnostic_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.diagnostic.as_ref())
            .and_then(|diagnostic| diagnostic.refresh_support)
            .unwrap_or(false);
        workspace.diagnostic_delivery =
            diagnostic::Delivery::new(pull_diagnostics, diagnostic_refresh);
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
                diagnostic_provider: pull_diagnostics.then(|| {
                    DiagnosticServerCapabilities::Options(DiagnosticOptions {
                        identifier: Some(diagnostic::IDENTIFIER.into()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: true,
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    })
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(
                        ["{", "[", ",", "<", "(", "|", "*", "+"]
                            .into_iter()
                            .map(str::to_owned)
                            .collect(),
                    ),
                    retrigger_characters: Some(
                        ["}", "]", ">", ")", "|"]
                            .into_iter()
                            .map(str::to_owned)
                            .collect(),
                    ),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(
                    true,
                )),
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
                        "latex.rebuild".into(),
                        "latex.cancelBuild".into(),
                        "latex.buildStatus".into(),
                        "latex.forwardSearch".into(),
                        "latex.inverseSearch".into(),
                        "latex.applyQuickFix".into(),
                    ],
                    ..Default::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                color_provider: Some(ColorProviderCapability::Simple(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(semantic::options()),
                ),
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
                        did_create: Some(file_operation_options()),
                        did_delete: Some(file_operation_options()),
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
            let binary_inputs = syntax::file_extensions("includegraphics")
                .unwrap()
                .iter()
                .chain(recorder::DATA_EXTENSIONS)
                .map(|extension| {
                    extension
                        .chars()
                        .flat_map(|ch| ['[', ch, ch.to_ascii_uppercase(), ']'])
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(",");
            let registration = Registration {
                id: "latex-lsp-watch-files".into(),
                method: "workspace/didChangeWatchedFiles".into(),
                register_options: Some(serde_json::json!({
                    "watchers": [
                        {"globPattern": format!("**/*.{{{}}}",workspace::SOURCE_EXTENSIONS.join(",")), "kind": 7},
                        {"globPattern": format!("**/*.{{{binary_inputs}}}"), "kind": 7},
                        {"globPattern": "**/*", "kind": 5}
                    ]
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
        self.cleanup().await;
        self.shutdown_requested.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        self.workspace
            .write()
            .await
            .open(item.uri.clone(), item.text, item.version);
        self.publish_open().await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = document_uri(params.text_document.uri);
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
        let kind_hint = doc.kind_hint;
        workspace.documents.insert(
            uri.clone(),
            Document::new_with_kind(
                &uri,
                text.source,
                Some(params.text_document.version),
                kind_hint,
            ),
        );
        workspace.clear_build_diagnostics(&uri);
        workspace.load_dependencies(&uri, 0);
        drop(workspace);
        self.publish_open().await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = document_uri(params.text_document.uri);
        self.publish_open().await;
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
        let uri = document_uri(params.text_document.uri);
        let mut workspace = self.workspace.write().await;
        workspace.close(&uri);
        self.semantic_cache.lock().await.remove(&uri);
        self.diagnostic_cache.lock().await.remove(&uri);
        let delivery = workspace.diagnostic_delivery.clone();
        drop(workspace);
        if !delivery.is_pull() {
            delivery
                .publish(&self.client, vec![(uri, vec![], None)])
                .await;
        }
        self.publish_open().await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(
                params.changes.iter().map(|change| change.uri.as_str()),
            ),
            true,
        )
        .await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        match config::Config::parse(params.settings) {
            Ok(config) => {
                self.workspace.write().await.config = config;
                self.publish_open().await;
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
            .map(document_uri)
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(
                    "Expected a document URI as the first command argument",
                )
            })?;
        match params.command.as_str() {
            "latex.applyQuickFix" => {
                let mut request: editing::FixRequest =
                    serde_json::from_value(argument.cloned().unwrap_or_default()).map_err(
                        |_| tower_lsp::jsonrpc::Error::invalid_params("Invalid quick fix request"),
                    )?;
                request.uri = document_uri(request.uri);
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
            "latex.rebuild" => self.builds.rebuild(uri).await.map(Some),
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

    async fn did_create_files(&self, params: CreateFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(params.files.iter().map(|file| file.uri.as_str())),
            false,
        )
        .await;
    }

    async fn did_delete_files(&self, params: DeleteFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(params.files.iter().map(|file| file.uri.as_str())),
            false,
        )
        .await;
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
                drop(workspace);
                self.publish_affected(affected).await;
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
        drop(workspace);
        self.publish_open().await;
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        validate_diagnostic_identifier(params.identifier.as_deref())?;
        let uri = document_uri(params.text_document.uri);
        let workspace = self.workspace.read().await;
        let snapshot = workspace.diagnostic_snapshots([uri]).pop().unwrap();
        drop(workspace);
        let report = self
            .diagnostic_cache
            .lock()
            .await
            .report(snapshot, params.previous_result_id.as_deref());
        let report = match report {
            DocumentDiagnosticReportKind::Full(full_document_diagnostic_report) => {
                DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                    related_documents: None,
                    full_document_diagnostic_report,
                })
            }
            DocumentDiagnosticReportKind::Unchanged(unchanged_document_diagnostic_report) => {
                DocumentDiagnosticReport::Unchanged(RelatedUnchangedDocumentDiagnosticReport {
                    related_documents: None,
                    unchanged_document_diagnostic_report,
                })
            }
        };
        Ok(report.into())
    }

    async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult> {
        validate_diagnostic_identifier(params.identifier.as_deref())?;
        let previous: std::collections::BTreeMap<_, _> = params
            .previous_result_ids
            .into_iter()
            .map(|previous| (document_uri(previous.uri), previous.value))
            .collect();
        let workspace = self.workspace.read().await;
        let uris = workspace
            .documents
            .keys()
            .cloned()
            .chain(
                workspace
                    .build_diagnostics
                    .values()
                    .flat_map(|project| project.keys().cloned()),
            )
            // Previously reported files may have been removed or renamed. Send
            // empty reports for them so the client clears their diagnostics.
            .chain(previous.keys().cloned());
        let snapshots = workspace.diagnostic_snapshots(uris);
        drop(workspace);
        let mut cache = self.diagnostic_cache.lock().await;
        let items = snapshots
            .into_iter()
            .map(|snapshot| {
                let uri = snapshot.0.clone();
                let version = snapshot.2.map(i64::from);
                match cache.report(snapshot, previous.get(&uri).map(String::as_str)) {
                    DocumentDiagnosticReportKind::Full(full_document_diagnostic_report) => {
                        WorkspaceDocumentDiagnosticReport::Full(
                            WorkspaceFullDocumentDiagnosticReport {
                                uri,
                                version,
                                full_document_diagnostic_report,
                            },
                        )
                    }
                    DocumentDiagnosticReportKind::Unchanged(
                        unchanged_document_diagnostic_report,
                    ) => WorkspaceDocumentDiagnosticReport::Unchanged(
                        WorkspaceUnchangedDocumentDiagnosticReport {
                            uri,
                            version,
                            unchanged_document_diagnostic_report,
                        },
                    ),
                }
            })
            .collect();
        Ok(WorkspaceDiagnosticReport { items }.into())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        Ok(Some(CompletionResponse::Array(features::completion(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
        ))))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params;
        Ok(features::hover(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
        ))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let params = params.text_document_position_params;
        Ok(signature::help(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.position,
        ))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let position = params.text_document_position_params;
        let locations = features::definitions(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
        );
        Ok(Some(GotoDefinitionResponse::Array(locations)))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let position = params.text_document_position;
        Ok(Some(features::references(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
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
            &document_uri(params.text_document.uri),
            params.position,
        )
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let position = params.text_document_position;
        features::rename(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
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
            &document_uri(params.text_document.uri),
            params.position,
        )))
    }

    async fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> Result<Option<LinkedEditingRanges>> {
        let params = params.text_document_position_params;
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace
            .documents
            .get(&document_uri(params.text_document.uri))
        else {
            return Ok(None);
        };
        if doc.text.offset(params.position).is_none() {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Invalid linked editing position",
            ));
        }
        Ok(editing::linked_editing_ranges(doc, params.position))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace
            .documents
            .get(&document_uri(params.text_document.uri))
        else {
            return Ok(None);
        };
        editing::selection_ranges(doc, &params.positions)
            .map(Some)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Invalid selection position"))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        Ok(Some(editing::code_actions(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.range,
            &params.context,
        )))
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        Ok(Some(code_lens::lenses(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
        )))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let workspace = self.workspace.read().await;
        let uri = document_uri(params.text_document.uri);
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
            .get(&document_uri(params.text_document.uri))
            .map(|doc| navigation::folding_ranges(doc, workspace.folding_limit)))
    }

    async fn document_color(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        let workspace = self.workspace.read().await;
        Ok(workspace
            .documents
            .get(&document_uri(params.text_document.uri))
            .map(colors::document_colors)
            .unwrap_or_default())
    }

    async fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> Result<Vec<ColorPresentation>> {
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace
            .documents
            .get(&document_uri(params.text_document.uri))
        else {
            return Ok(vec![]);
        };
        if params.range.start > params.range.end
            || doc.text.offset(params.range.start).is_none()
            || doc.text.offset(params.range.end).is_none()
        {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Invalid color presentation range",
            ));
        }
        Ok(colors::presentations(doc, params.range, params.color))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let workspace = self.workspace.read().await;
        let uri = document_uri(params.text_document.uri);
        let Some(doc) = workspace.documents.get(&uri) else {
            return Ok(None);
        };
        let tokens = semantic::tokens(doc, None)?;
        Ok(Some(SemanticTokensResult::Tokens(
            self.semantic_cache.lock().await.full(uri, tokens),
        )))
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        let workspace = self.workspace.read().await;
        let uri = document_uri(params.text_document.uri);
        let Some(doc) = workspace.documents.get(&uri) else {
            return Ok(None);
        };
        let tokens = semantic::tokens(doc, None)?;
        Ok(Some(self.semantic_cache.lock().await.delta(
            uri,
            &params.previous_result_id,
            tokens,
        )))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let workspace = self.workspace.read().await;
        workspace
            .documents
            .get(&document_uri(params.text_document.uri))
            .map(|doc| {
                semantic::tokens(doc, Some(params.range)).map(SemanticTokensRangeResult::Tokens)
            })
            .transpose()
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        Ok(Some(navigation::document_links(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
        )))
    }
}
