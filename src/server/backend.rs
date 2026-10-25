//! LSP backend and protocol handlers.
use crate::{
    compilation::{auxiliary, build, recorder, synctex},
    config::{self, project_config},
    diagnostics::{chktex, diagnostic, diagnostic_inputs},
    editing::{self, code_action_resolve, file_operations, refactor},
    formatting::{self, bib_formatting, on_type_formatting, range_formatting},
    language::{
        call_hierarchy, code_lens, colors, completion_resolve, features, navigation, semantic,
        semantic_cache, signature,
    },
    parser::syntax,
    project::distribution,
    server::{dispatch, tasks},
};

use crate::project::workspace::{self, document_uri, Workspace};
use crate::rpc::Result;
use crate::server::client::Client;
use lsp_types::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{Mutex, RwLock};

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
        Err(crate::rpc::Error::invalid_params(
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
    external_tasks: tasks::Tasks,
    shutdown_requested: AtomicBool,
    semantic_cache: Mutex<semantic_cache::Cache>,
    diagnostic_cache: Arc<Mutex<diagnostic::Cache>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let workspace = Arc::new(RwLock::new(Workspace::default()));
        Self {
            builds: build::BuildManager::new(client.clone(), workspace.clone()),
            sync_queries: synctex::Queries::default(),
            external_tasks: tasks::Tasks::default(),
            client,
            workspace,
            shutdown_requested: AtomicBool::new(false),
            semantic_cache: Mutex::new(semantic_cache::Cache::default()),
            diagnostic_cache: Arc::new(Mutex::new(diagnostic::Cache::default())),
        }
    }

    /// Cancel and reap external work, retaining final notifications for shutdown.
    pub fn cleanup(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let builds = self.builds.clone();
        let queries = self.sync_queries.clone();
        let external_tasks = self.external_tasks.clone();
        let workspace = self.workspace.clone();
        async move {
            tokio::join!(queries.shutdown(), external_tasks.shutdown());
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

    /// Keep CPU-heavy reads off the transport task so a completed external
    /// query can return while annotations are being calculated. Release the
    /// dispatch turn after acquiring the read guard: other reads can proceed,
    /// while the guard keeps subsequent document writes in wire order. A queued
    /// writer also prevents later reads from overtaking that document update.
    async fn read_query<T: Send + 'static>(
        &self,
        query: impl FnOnce(&Workspace) -> T + Send + 'static,
    ) -> Result<T> {
        let workspace = self.workspace.clone().read_owned().await;
        dispatch::release();
        tokio::task::spawn_blocking(move || query(&workspace))
            .await
            .map_err(|error| {
                crate::rpc::Error::invalid_params(format!("Analysis task failed: {error}"))
            })
    }

    async fn reload_distribution(&self) {
        let config = self.workspace.read().await.config.distribution.clone();
        let discovery_config = config.clone();
        if let Ok(mut task) = self.external_tasks.start(move |cancel| {
            let distribution =
                distribution::Distribution::discover_cancelled(&discovery_config, || {
                    cancel.cancelled()
                });
            if cancel.cancelled() {
                return Err(crate::rpc::Error::request_cancelled());
            }
            Ok(distribution)
        }) {
            if let Ok(distribution) = task.wait().await {
                let mut workspace = self.workspace.write().await;
                workspace.diagnostic_delivery.invalidate();
                if workspace.config.distribution == config {
                    workspace.distribution = distribution;
                }
            }
        }
    }

    async fn format_document(
        &self,
        uri: Url,
        options: FormattingOptions,
        range: Option<Range>,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = document_uri(uri);
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace.documents.get(&uri) else {
            return Ok(None);
        };
        let kind = doc.kind;
        if !matches!(
            kind,
            workspace::DocumentKind::Latex | workspace::DocumentKind::Bibtex
        ) {
            return Ok(None);
        }
        let range = range
            .map(|range| range_formatting::normalize(&doc.text, range))
            .transpose()?;
        if let Some(range) = range {
            formatting::validate_options(&options)?;
            if range.start == range.end {
                return Ok(Some(vec![]));
            }
        }
        let text = doc.text.clone();
        let version = doc.version;
        let config = workspace.settings(&uri).formatting.clone();
        let directory = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .filter(|path| path.is_dir())
            .or_else(|| workspace.roots.iter().next().cloned());
        let mut task = if kind == workspace::DocumentKind::Bibtex {
            bib_formatting::start(&self.external_tasks, options, text.source.clone(), range)?
        } else {
            formatting::start(
                &self.external_tasks,
                config.clone(),
                options,
                text.source.clone(),
                directory,
            )?
        };
        drop(workspace);
        dispatch::release();
        let formatted = task.wait().await?;
        let workspace = self.workspace.read().await;
        if workspace.settings(&uri).formatting != config
            || !workspace.documents.get(&uri).is_some_and(|doc| {
                doc.version == version && doc.text.source == text.source && doc.kind == kind
            })
        {
            return Err(crate::rpc::Error {
                code: crate::rpc::ErrorCode::ServerError(-32801),
                message: "Document or formatting configuration changed; request formatting again"
                    .into(),
                data: None,
            });
        }
        drop(workspace);
        Ok(Some(match range {
            _ if kind == workspace::DocumentKind::Bibtex => {
                formatting::replacement(&text, 0..text.source.len(), &formatted)
                    .into_iter()
                    .collect()
            }
            Some(range) if kind == workspace::DocumentKind::Latex => {
                range_formatting::edits(&text, formatted, range)
            }
            _ => formatting::edits(&text, formatted),
        }))
    }

    async fn reindex(&self) -> Result<serde_json::Value> {
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
        let state = Arc::new(std::sync::Mutex::new(Some(std::mem::take(&mut *workspace))));
        let input = state.clone();
        let task = self.external_tasks.start(move |cancel| {
            let mut indexed = input.lock().unwrap().take().unwrap();
            let roots = std::mem::take(&mut indexed.roots);
            indexed.documents.retain(|_, doc| doc.version.is_some());
            indexed.index_truncated.clear();
            for root in roots {
                indexed.add_root_in_background(root, &|| cancel.cancelled());
            }
            Ok(indexed)
        });
        let result = match task {
            Ok(mut task) => task.wait().await,
            Err(error) => Err(error),
        };
        match result {
            Ok(indexed) => *workspace = indexed,
            Err(error) => {
                if let Some(original) = state.lock().unwrap().take() {
                    *workspace = original;
                }
                return Err(error);
            }
        }
        Ok(
            serde_json::json!({"files":workspace.documents.len(),"truncated":workspace.index_truncated,"limits":workspace.config.indexing}),
        )
    }

    async fn lint(&self, uri: &Url) {
        let workspace = self.workspace.read().await;
        let config = workspace.settings(uri).diagnostics.chktex.clone();
        let Some(doc) = workspace
            .documents
            .get(uri)
            .filter(|doc| config.enabled && doc.kind == workspace::DocumentKind::Latex)
        else {
            return;
        };
        let (source, version) = (doc.text.source.clone(), doc.version);
        let directory = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        let task = chktex::start(
            &self.external_tasks,
            config.clone(),
            source.clone(),
            directory,
        );
        drop(workspace);
        dispatch::release();
        let result = match task {
            Ok(mut task) => task.wait().await,
            Err(error) => Err(error),
        };
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
        if workspace.settings(uri).diagnostics.chktex != config
            || !workspace.documents.get(uri).is_some_and(|doc| {
                doc.version == version && doc.version.is_some() && doc.text.source == source
            })
        {
            return;
        }
        match result {
            Ok(diagnostics) => {
                workspace.lint_diagnostics.insert(uri.clone(), diagnostics);
            }
            Err(error) => {
                workspace.lint_diagnostics.remove(uri);
                drop(workspace);
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
                self.publish_open().await;
                return;
            }
        }
        drop(workspace);
        self.publish_open().await;
    }

    async fn publish_open(&self) {
        self.publish_affected(Default::default()).await;
    }

    async fn diagnostic_reports(
        &self,
        workspace: tokio::sync::OwnedRwLockReadGuard<Workspace>,
        uris: std::collections::BTreeSet<Url>,
        previous: std::collections::BTreeMap<Url, String>,
    ) -> Result<Vec<(Url, Option<i32>, DocumentDiagnosticReportKind)>> {
        dispatch::release();
        let mut cache = self.diagnostic_cache.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let revision = workspace.diagnostic_delivery.revision();
            let mut reports = std::collections::BTreeMap::new();
            let missing: Vec<_> = uris
                .into_iter()
                .filter(|uri| {
                    if let Some(report) =
                        cache.lookup(uri, previous.get(uri).map(String::as_str), revision)
                    {
                        reports.insert(uri.clone(), report);
                        false
                    } else {
                        true
                    }
                })
                .collect();
            if !missing.is_empty() {
                let (snapshots, files) =
                    diagnostic_inputs::Files::capture(|| workspace.diagnostic_snapshots(missing));
                for snapshot in snapshots {
                    let uri = snapshot.0.clone();
                    let report = cache.computed(
                        snapshot,
                        previous.get(&uri).map(String::as_str),
                        Some(diagnostic::Inputs {
                            revision,
                            files: files.clone(),
                        }),
                    );
                    reports.insert(uri, report);
                }
            }
            reports
                .into_iter()
                .map(|(uri, report)| {
                    let version = workspace.documents.get(&uri).and_then(|doc| doc.version);
                    (uri, version, report)
                })
                .collect()
        })
        .await
        .map_err(|error| {
            crate::rpc::Error::invalid_params(format!("Analysis task failed: {error}"))
        })
    }

    async fn publish_affected(&self, affected: std::collections::BTreeSet<Url>) {
        let workspace = self.workspace.read().await;
        let delivery = workspace.diagnostic_delivery.clone();
        if delivery.is_pull() {
            drop(workspace);
            delivery.publish(&self.client, vec![]).await;
        } else {
            delivery.schedule(&self.client, self.workspace.clone(), affected);
        }
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
            workspace.diagnostic_delivery.invalidate();
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
        // Revalidate cached analysis after a watcher event, preserving result
        // IDs when a delayed notification changes neither payload nor version.
        self.publish_affected(affected).await;
    }
}

impl Backend {
    pub(crate) async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
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
            workspace.config =
                config::Config::parse(options).map_err(crate::rpc::Error::invalid_params)?;
        }
        workspace.inlay_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|cap| cap.inlay_hint.as_ref())
            .and_then(|cap| cap.refresh_support)
            .unwrap_or(false);
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
        workspace.completion_resolve_documentation = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.completion.as_ref())
            .and_then(|completion| completion.completion_item.as_ref())
            .and_then(|item| item.resolve_support.as_ref())
            .is_some_and(|support| {
                support
                    .properties
                    .iter()
                    .any(|property| property == "documentation")
            });
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
        workspace.code_action_resolve_edits = workspace.code_action_literals
            && params
                .capabilities
                .text_document
                .as_ref()
                .and_then(|doc| doc.code_action.as_ref())
                .is_some_and(|action| {
                    action.data_support == Some(true)
                        && action.resolve_support.as_ref().is_some_and(|support| {
                            support.properties.iter().any(|property| property == "edit")
                        })
                });
        workspace.folding_capabilities = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|doc| doc.folding_range.as_ref())
            .cloned()
            .unwrap_or_default();
        let mut scan_roots = Vec::new();
        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                if let Ok(root) = folder.uri.to_file_path() {
                    scan_roots.push(root);
                }
            }
        }
        if scan_roots.is_empty() {
            if let Some(uri) = params.root_uri {
                if let Ok(root) = uri.to_file_path() {
                    scan_roots.push(root);
                }
            }
        }
        let mut indexed = std::mem::take(&mut *workspace);
        let mut task = self.external_tasks.start(move |cancel| {
            for root in scan_roots {
                if cancel.cancelled() {
                    return Err(crate::rpc::Error::request_cancelled());
                }
                indexed.add_root_in_background(root, &|| cancel.cancelled());
            }
            Ok(indexed)
        })?;
        *workspace = task.wait().await?;
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
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        "\\".into(),
                        "{".into(),
                        ",".into(),
                        "/".into(),
                        "@".into(),
                        "[".into(),
                        "=".into(),
                    ]),
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
                inlay_hint_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                    first_trigger_character: "\n".into(),
                    more_trigger_character: Some(vec!["}".into()]),
                }),
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
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(
                    true,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_REWRITE,
                            CodeActionKind::REFACTOR_EXTRACT,
                            CodeActionKind::SOURCE_FIX_ALL,
                        ]),
                        resolve_provider: Some(true),
                        ..Default::default()
                    },
                )),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "latex.indexingStatus".into(),
                        "latex.reindex".into(),
                        "latex.build".into(),
                        "latex.rebuild".into(),
                        "latex.cleanAuxiliary".into(),
                        "latex.cleanArtifacts".into(),
                        "latex.cancelBuild".into(),
                        "latex.buildStatus".into(),
                        "latex.forwardSearch".into(),
                        "latex.inverseSearch".into(),
                        "latex.applyQuickFix".into(),
                        "latex.applyCodeAction".into(),
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

    pub(crate) async fn initialized(&self, _: InitializedParams) {
        // Distribution discovery and watcher registration do not alter source
        // text. Let document notifications and interactive requests proceed
        // while waiting for the external tool or the client's registration ACK.
        dispatch::release();
        self.reload_distribution().await;
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
                        {"globPattern": "**/{.latexmkrc,latexmkrc,.latex-lsp.json}", "kind": 7},
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

    pub(crate) async fn shutdown(&self) -> Result<()> {
        self.cleanup().await;
        self.shutdown_requested.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
        workspace.open_with_language(item.uri, item.text, item.version, &item.language_id);
        drop(workspace);
        self.publish_open().await;
    }

    pub(crate) async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = document_uri(params.text_document.uri);
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
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
        let updated = doc.with_text(&uri, text.source, Some(params.text_document.version));
        workspace.documents.insert(uri.clone(), updated);
        workspace.clear_build_diagnostics(&uri);
        workspace.load_dependencies(&uri, 0);
        drop(workspace);
        self.publish_open().await;
    }

    pub(crate) async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = document_uri(params.text_document.uri);
        self.publish_open().await;
        let on_save = self.workspace.read().await.settings(&uri).build.on_save;
        if on_save {
            if let Err(error) = self.builds.on_save(uri.clone()).await {
                self.client
                    .log_message(MessageType::WARNING, error.message)
                    .await;
            }
        }
        self.lint(&uri).await;
    }

    pub(crate) async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = document_uri(params.text_document.uri);
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
        workspace.close(&uri);
        workspace.lint_diagnostics.remove(&uri);
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

    pub(crate) async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(
                params.changes.iter().map(|change| change.uri.as_str()),
            ),
            true,
        )
        .await;
    }

    pub(crate) async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        match config::Config::parse(params.settings) {
            Ok(config) => {
                let indexing_changed =
                    self.workspace.read().await.config.indexing != config.indexing;
                let distribution_changed =
                    self.workspace.read().await.config.distribution != config.distribution;
                {
                    let mut workspace = self.workspace.write().await;
                    workspace.diagnostic_delivery.invalidate();
                    workspace.lint_diagnostics.clear();
                    project_config::invalidate();
                    workspace.config = config;
                }
                if indexing_changed {
                    if let Err(error) = self.reindex().await {
                        self.client
                            .log_message(MessageType::WARNING, error.message)
                            .await;
                    }
                }
                if distribution_changed {
                    self.reload_distribution().await;
                }
                self.publish_open().await;
            }
            Err(error) => self.client.log_message(MessageType::WARNING, error).await,
        }
    }

    pub(crate) async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        if params.command == "latex.reindex" {
            let status = self.reindex().await?;
            self.publish_open().await;
            return Ok(Some(status));
        }
        if params.command == "latex.indexingStatus" {
            let workspace = self.workspace.read().await;
            return Ok(Some(
                serde_json::json!({"files":workspace.documents.len(),"truncated":workspace.index_truncated,"limits":workspace.config.indexing}),
            ));
        }
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
                crate::rpc::Error::invalid_params(
                    "Expected a document URI as the first command argument",
                )
            })?;
        match params.command.as_str() {
            "latex.applyCodeAction" => {
                let mut request: refactor::Request = serde_json::from_value(
                    argument.cloned().unwrap_or_default(),
                )
                .map_err(|_| crate::rpc::Error::invalid_params("Invalid code action request"))?;
                request.uri = document_uri(request.uri);
                let edit =
                    refactor::resolve(&*self.workspace.read().await, request).ok_or_else(|| {
                        crate::rpc::Error::invalid_params(
                            "Code action is outdated; request code actions again",
                        )
                    })?;
                let response = self.client.apply_edit(edit).await?;
                Ok(Some(serde_json::to_value(response).unwrap()))
            }
            "latex.applyQuickFix" => {
                let mut request: editing::FixRequest = serde_json::from_value(
                    argument.cloned().unwrap_or_default(),
                )
                .map_err(|_| crate::rpc::Error::invalid_params("Invalid quick fix request"))?;
                request.uri = document_uri(request.uri);
                let edit = editing::resolve_fix(&*self.workspace.read().await, request)
                    .ok_or_else(|| {
                        crate::rpc::Error::invalid_params(
                            "Quick fix is outdated; request code actions again",
                        )
                    })?;
                let response = self.client.apply_edit(edit).await?;
                Ok(Some(serde_json::to_value(response).unwrap()))
            }
            "latex.build" => self.builds.start(uri).await.map(Some),
            "latex.cleanAuxiliary" | "latex.cleanArtifacts" => {
                let dry_run = match argument.and_then(|value| value.get("dryRun")) {
                    None => false,
                    Some(value) => value.as_bool().ok_or_else(|| {
                        crate::rpc::Error::invalid_params("dryRun must be a boolean")
                    })?,
                };
                self.builds
                    .clean(uri, params.command == "latex.cleanArtifacts", dry_run)
                    .await
                    .map(Some)
            }
            "latex.rebuild" => {
                if let Some(value) = argument.and_then(|value| value.get("rebuildGuard")) {
                    let mut guard: editing::RebuildGuard = serde_json::from_value(value.clone())
                        .map_err(|_| {
                            crate::rpc::Error::invalid_params(
                                "Invalid rebuild action; request code actions again",
                            )
                        })?;
                    guard.uri = document_uri(guard.uri);
                    self.builds.rebuild_guarded(uri, guard).await.map(Some)
                } else {
                    self.builds.rebuild(uri).await.map(Some)
                }
            }
            "latex.forwardSearch" | "latex.inverseSearch" => {
                let project = synctex::prepare(&*self.workspace.read().await, &uri)?;
                let available = |root: &Url| {
                    let status = self.builds.status(root);
                    status["status"] == "idle"
                        || (status["status"] == "success" && status["stale"] != true)
                };
                if !available(&project.root) {
                    return Err(crate::rpc::Error::invalid_params(
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
                    return Err(crate::rpc::Error::invalid_params(
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
                    .map_err(crate::rpc::Error::invalid_params)?;
                if params.command == "latex.cancelBuild" {
                    Ok(Some(
                        serde_json::json!({"cancelled":self.builds.cancel(Some(&root))}),
                    ))
                } else {
                    Ok(Some(self.builds.status(&root)))
                }
            }
            _ => Err(crate::rpc::Error::invalid_params(format!(
                "Unknown command: {}",
                params.command
            ))),
        }
    }

    pub(crate) async fn will_rename_files(
        &self,
        params: RenameFilesParams,
    ) -> Result<Option<WorkspaceEdit>> {
        file_operations::will_rename(&*self.workspace.read().await, &params.files)
    }

    pub(crate) async fn did_create_files(&self, params: CreateFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(params.files.iter().map(|file| file.uri.as_str())),
            false,
        )
        .await;
    }

    pub(crate) async fn did_delete_files(&self, params: DeleteFilesParams) {
        self.refresh_file_events(
            file_operations::FileChanges::new(params.files.iter().map(|file| file.uri.as_str())),
            false,
        )
        .await;
    }

    pub(crate) async fn did_rename_files(&self, params: RenameFilesParams) {
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
        workspace.diagnostic_delivery.invalidate();
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

    pub(crate) async fn did_change_workspace_folders(
        &self,
        params: DidChangeWorkspaceFoldersParams,
    ) {
        let mut workspace = self.workspace.write().await;
        workspace.diagnostic_delivery.invalidate();
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

    pub(crate) async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        validate_diagnostic_identifier(params.identifier.as_deref())?;
        let uri = document_uri(params.text_document.uri);
        let workspace = self.workspace.clone().read_owned().await;
        let previous = params
            .previous_result_id
            .map(|previous| (uri.clone(), previous))
            .into_iter()
            .collect();
        let (_, _, report) = self
            .diagnostic_reports(workspace, [uri].into_iter().collect(), previous)
            .await?
            .pop()
            .unwrap();
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

    pub(crate) async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult> {
        validate_diagnostic_identifier(params.identifier.as_deref())?;
        let previous: std::collections::BTreeMap<_, _> = params
            .previous_result_ids
            .into_iter()
            .map(|previous| (document_uri(previous.uri), previous.value))
            .collect();
        let workspace = self.workspace.clone().read_owned().await;
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
            // Clear previously reported files which were removed.
            .chain(previous.keys().cloned())
            .collect();
        let reports = self.diagnostic_reports(workspace, uris, previous).await?;
        let items = reports
            .into_iter()
            .map(|(uri, version, report)| {
                let version = version.map(i64::from);
                match report {
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

    pub(crate) async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        self.format_document(params.text_document.uri, params.options, None)
            .await
    }

    pub(crate) async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        self.format_document(params.text_document.uri, params.options, Some(params.range))
            .await
    }

    pub(crate) async fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = document_uri(params.text_document_position.text_document.uri);
        let workspace = self.workspace.read().await;
        let Some(doc) = workspace.documents.get(&uri) else {
            return Ok(None);
        };
        if doc.kind != workspace::DocumentKind::Latex {
            return Ok(None);
        }
        if !workspace.settings(&uri).formatting.on_type {
            return Ok(Some(vec![]));
        }
        on_type_formatting::edits(
            &workspace,
            &uri,
            params.text_document_position.position,
            &params.ch,
            &params.options,
        )
        .map(Some)
    }

    pub(crate) async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let params = params.text_document_position_params;
        call_hierarchy::prepare(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.position,
        )
    }

    pub(crate) async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        call_hierarchy::incoming(&*self.workspace.read().await, &params.item).map(Some)
    }

    pub(crate) async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        call_hierarchy::outgoing(&*self.workspace.read().await, &params.item).map(Some)
    }

    pub(crate) async fn inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> Result<Option<Vec<InlayHint>>> {
        if params.range.start > params.range.end {
            return Err(crate::rpc::Error::invalid_params(
                "Invalid inlay hint range",
            ));
        }
        Ok(Some(auxiliary::hints(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.range,
        )))
    }

    pub(crate) async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        let workspace = self.workspace.read().await;
        let mut items = features::completion(
            &workspace,
            &document_uri(position.text_document.uri),
            position.position,
        );
        let limit = workspace.config.indexing.completion_limit;
        let incomplete = items.len() >= limit || !workspace.index_truncated.is_empty();
        items.truncate(limit);
        Ok(Some(if incomplete {
            CompletionResponse::List(CompletionList {
                is_incomplete: true,
                items,
            })
        } else {
            CompletionResponse::Array(items)
        }))
    }

    pub(crate) async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        completion_resolve::resolve(&*self.workspace.read().await, item)
    }

    pub(crate) async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params;
        Ok(features::hover(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
        ))
    }

    pub(crate) async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
        let params = params.text_document_position_params;
        Ok(signature::help(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.position,
        ))
    }

    pub(crate) async fn goto_definition(
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

    pub(crate) async fn references(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let position = params.text_document_position;
        Ok(Some(features::references(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
            params.context.include_declaration,
        )))
    }

    pub(crate) async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        features::prepare_rename(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
            params.position,
        )
    }

    pub(crate) async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let position = params.text_document_position;
        features::rename(
            &*self.workspace.read().await,
            &document_uri(position.text_document.uri),
            position.position,
            &params.new_name,
        )
    }

    pub(crate) async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let params = params.text_document_position_params;
        self.read_query(move |workspace| {
            editing::highlights(
                workspace,
                &document_uri(params.text_document.uri),
                params.position,
            )
        })
        .await
        .map(Some)
    }

    pub(crate) async fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> Result<Option<LinkedEditingRanges>> {
        let params = params.text_document_position_params;
        self.read_query(move |workspace| {
            let uri = document_uri(params.text_document.uri);
            let Some(doc) = workspace.documents.get(&uri) else {
                return Ok(None);
            };
            if doc.text.offset(params.position).is_none() {
                return Err(crate::rpc::Error::invalid_params(
                    "Invalid linked editing position",
                ));
            }
            Ok(editing::linked_editing_ranges_in_workspace(
                workspace,
                &uri,
                params.position,
            ))
        })
        .await?
    }

    pub(crate) async fn selection_range(
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
            .ok_or_else(|| crate::rpc::Error::invalid_params("Invalid selection position"))
    }

    pub(crate) async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        self.read_query(move |workspace| {
            code_action_resolve::actions(
                workspace,
                &document_uri(params.text_document.uri),
                params.range,
                &params.context,
            )
        })
        .await
        .map(Some)
    }

    pub(crate) async fn code_action_resolve(&self, action: CodeAction) -> Result<CodeAction> {
        code_action_resolve::resolve(&*self.workspace.read().await, action)
    }

    pub(crate) async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        Ok(Some(code_lens::lenses(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
        )))
    }

    pub(crate) async fn document_symbol(
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

    pub(crate) async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        Ok(Some(navigation::workspace_symbols(
            &*self.workspace.read().await,
            &params.query,
        )))
    }

    pub(crate) async fn folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> Result<Option<Vec<FoldingRange>>> {
        let workspace = self.workspace.read().await;
        Ok(workspace
            .documents
            .get(&document_uri(params.text_document.uri))
            .map(|doc| navigation::folding_ranges_for_client(doc, &workspace.folding_capabilities)))
    }

    pub(crate) async fn document_color(
        &self,
        params: DocumentColorParams,
    ) -> Result<Vec<ColorInformation>> {
        self.read_query(move |workspace| {
            workspace
                .documents
                .get(&document_uri(params.text_document.uri))
                .map(colors::document_colors)
                .unwrap_or_default()
        })
        .await
    }

    pub(crate) async fn color_presentation(
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
            return Err(crate::rpc::Error::invalid_params(
                "Invalid color presentation range",
            ));
        }
        Ok(colors::presentations(doc, params.range, params.color))
    }

    pub(crate) async fn semantic_tokens_full(
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

    pub(crate) async fn semantic_tokens_full_delta(
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

    pub(crate) async fn semantic_tokens_range(
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

    pub(crate) async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> Result<Option<Vec<DocumentLink>>> {
        Ok(Some(navigation::document_links(
            &*self.workspace.read().await,
            &document_uri(params.text_document.uri),
        )))
    }
}

#[cfg(test)]
mod read_query_tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn slow_analysis_allows_reads_but_keeps_writes_and_later_reads_in_order() {
        let (client, _outgoing) = Client::new();
        let service = Arc::new(Backend::new(client));
        let mut queue = dispatch::RequestQueue::default();
        let (started, start) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let first_service = service.clone();
        let first = tokio::spawn(queue.ordered(async move {
            first_service
                .read_query(move |workspace| {
                    let _ = started.send(());
                    let _ = finished.blocking_recv();
                    workspace.snippets
                })
                .await
                .unwrap()
        }));
        start.await.unwrap();

        let read_service = service.clone();
        let read = queue.ordered(async move {
            read_service
                .read_query(|workspace| workspace.snippets)
                .await
                .unwrap()
        });
        let result = tokio::time::timeout(Duration::from_secs(1), read).await;
        // Unblock the worker even if the assertion fails.
        if result.is_err() {
            let _ = finish.send(());
            panic!("An unrelated read waited for the slow analysis");
        }
        assert!(!result.unwrap());

        let (writing, write_started) = oneshot::channel();
        let writer_service = service.clone();
        let writer = tokio::spawn(queue.ordered(async move {
            let _ = writing.send(());
            writer_service.workspace.write().await.snippets = true;
        }));
        write_started.await.unwrap();
        let later = tokio::spawn(queue.ordered(async move {
            service
                .read_query(|workspace| workspace.snippets)
                .await
                .unwrap()
        }));
        assert!(!writer.is_finished());
        assert!(!later.is_finished());
        let _ = finish.send(());
        assert!(!first.await.unwrap());
        writer.await.unwrap();
        assert!(later.await.unwrap());
    }

    #[tokio::test]
    async fn cancelling_analysis_keeps_its_read_guard_until_the_worker_finishes() {
        let (client, _outgoing) = Client::new();
        let backend = Arc::new(Backend::new(client));
        let mut queue = dispatch::RequestQueue::default();
        let (started, start) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let query_backend = backend.clone();
        let query = tokio::spawn(queue.ordered(async move {
            query_backend
                .read_query(move |_| {
                    let _ = started.send(());
                    let _ = finished.blocking_recv();
                })
                .await
        }));
        start.await.unwrap();
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
        let (writing, write_started) = oneshot::channel();
        let writer = tokio::spawn(queue.ordered(async move {
            let _ = writing.send(());
            backend.workspace.write().await.snippets = true;
        }));
        write_started.await.unwrap();
        assert!(!writer.is_finished());
        let _ = finish.send(());
        tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap();
    }
}
