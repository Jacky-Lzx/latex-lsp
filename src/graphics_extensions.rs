//! Ordered graphicx extension settings, shared by navigation and build checks.
use crate::{
    signature::{definition_body_spans, CommandSignatures},
    source_inputs, syntax,
    workspace::{resolve_file_path, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

type Setting = Option<Vec<String>>;

fn named_declaration(source: &str, commands: &[syntax::Command], quoted: &BTreeSet<usize>) -> bool {
    commands.iter().enumerate().any(|(index, command)| {
        if quoted.contains(&command.span.start) {
            return false;
        }
        let name = if command.name == "csuse" {
            command
                .argument
                .as_ref()
                .map(|argument| argument.value.as_str())
        } else if command.name == "csname" {
            syntax::literal_csname(source, commands, index).map(|span| &source[span])
        } else {
            None
        };
        name.is_some_and(|name| matches!(name, "DeclareGraphicsExtensions" | "DeclareGraphicsRule"))
    })
}

pub(crate) struct Index {
    defaults: Vec<String>,
    settings: Option<BTreeMap<(Url, usize), Setting>>,
}

impl Index {
    pub fn new(workspace: &Workspace, root: &Url) -> Self {
        let defaults: Vec<_> = syntax::file_extensions("includegraphics")
            .unwrap()
            .iter()
            .map(|extension| (*extension).to_owned())
            .collect();
        let scope = workspace.project_documents(root);
        let documents: Vec<_> = scope
            .iter()
            .filter_map(|uri| workspace.documents.get(uri).map(|doc| (uri, doc)))
            .filter(|(_, doc)| doc.kind == DocumentKind::Latex)
            .collect();
        let customized = documents.iter().any(|(_, doc)| {
            doc.syntax.commands.iter().any(|command| {
                matches!(
                    command.name.as_str(),
                    "DeclareGraphicsExtensions" | "DeclareGraphicsRule"
                )
            }) || doc.syntax.definitions.iter().any(|entry| {
                matches!(
                    entry.name.as_str(),
                    "DeclareGraphicsExtensions" | "DeclareGraphicsRule"
                )
            }) || named_declaration(&doc.text.source, &doc.syntax.commands, &BTreeSet::new())
        });
        let mut index = Self {
            defaults,
            settings: customized.then(BTreeMap::new),
        };
        if !customized {
            return index;
        }
        if documents.len() > 1024
            || documents
                .iter()
                .any(|(_, doc)| doc.text.source.len() > 2 * 1024 * 1024)
            || documents
                .iter()
                .map(|(_, doc)| doc.text.source.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            return index;
        }
        let signatures = CommandSignatures::source_context(
            documents
                .iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        if [
            "DeclareGraphicsExtensions",
            "DeclareGraphicsRule",
            "includegraphics",
            "begingroup",
            "endgroup",
            "bgroup",
            "egroup",
            "begin",
            "end",
        ]
        .iter()
        .any(|name| !signatures.command_unchanged(name))
            || documents.iter().any(|(_, doc)| {
                let positions = signatures.source_command_positions(&doc.text.source, &doc.syntax);
                let quoted = signatures.quoted_command_tokens(&doc.text.source, &doc.syntax);
                named_declaration(&doc.text.source, &doc.syntax.commands, &quoted)
                    || doc.syntax.commands.iter().any(|command| {
                        !quoted.contains(&command.span.start)
                            && (command.name == "DeclareGraphicsRule"
                                || command.name == "DeclareGraphicsExtensions"
                                    && !positions.contains(&command.span.start))
                    })
            })
        {
            return index;
        }
        let inputs = source_inputs::Index::new(workspace, root);
        if inputs.incomplete {
            return index;
        }
        let mut walk = Walk {
            workspace,
            root,
            signatures,
            inputs,
            settings: BTreeMap::new(),
            active: BTreeSet::new(),
            visited: BTreeSet::new(),
            packages: BTreeSet::new(),
            state: Some(index.defaults.clone()),
            groups: Vec::new(),
            conditional: 0,
            visits: 0,
            events: 0,
            bytes: 0,
            complete: true,
        };
        walk.source(root);
        // Recorder-only settings have no proven position in the source order.
        let ordered = documents.iter().all(|(uri, doc)| {
            walk.visited.contains(*uri) || {
                let quoted = walk
                    .signatures
                    .quoted_command_tokens(&doc.text.source, &doc.syntax);
                !doc.syntax.commands.iter().any(|command| {
                    command.name == "DeclareGraphicsExtensions"
                        && !quoted.contains(&command.span.start)
                })
            }
        });
        if walk.complete && ordered {
            index.settings = Some(walk.settings);
        }
        index
    }

    pub fn at(&self, uri: &Url, offset: usize) -> Option<Vec<&str>> {
        let extensions = match &self.settings {
            None => &self.defaults,
            Some(settings) => settings.get(&(uri.clone(), offset))?.as_ref()?,
        };
        Some(extensions.iter().map(String::as_str).collect())
    }
}

fn declaration(source: &str, command: &syntax::Command) -> Setting {
    let argument = command.argument.as_ref()?;
    if argument.outer_span.start != syntax::skip_space(source, command.span.end)
        || source.as_bytes().get(argument.outer_span.start) != Some(&b'{')
    {
        return None;
    }
    // TeX removes comments before zap@space removes ordinary spaces.
    let value = argument
        .value
        .lines()
        .map(|line| line.split('%').next().unwrap_or_default())
        .collect::<String>()
        .replace([' ', '\t', '\r'], "");
    let mut result = Vec::new();
    if value.is_empty() {
        return Some(result);
    }
    for extension in value.split(',') {
        let extension = extension.strip_prefix('.')?;
        if extension.is_empty()
            || extension.len() > 32
            || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return None;
        }
        if !result.iter().any(|value| value == extension) {
            result.push(extension.to_owned());
        }
        if result.len() > 64 {
            return None;
        }
    }
    Some(result)
}

struct Walk<'a> {
    workspace: &'a Workspace,
    root: &'a Url,
    signatures: CommandSignatures<'a>,
    inputs: source_inputs::Index,
    settings: BTreeMap<(Url, usize), Setting>,
    active: BTreeSet<Url>,
    visited: BTreeSet<Url>,
    packages: BTreeSet<Url>,
    state: Setting,
    groups: Vec<Setting>,
    conditional: usize,
    visits: usize,
    events: usize,
    bytes: usize,
    complete: bool,
}

impl Walk<'_> {
    fn source(&mut self, uri: &Url) {
        self.visits += 1;
        if self.visits > 4096 || self.active.len() >= 64 || !self.active.insert(uri.clone()) {
            self.complete = false;
            return;
        }
        let workspace = self.workspace;
        self.visited.insert(uri.clone());
        let Some(doc) = workspace.documents.get(uri) else {
            self.state = None;
            self.active.remove(uri);
            return;
        };
        self.bytes += doc.text.source.len();
        if self.bytes > 32 * 1024 * 1024 {
            self.complete = false;
            return;
        }
        let positions = self
            .signatures
            .source_command_positions(&doc.text.source, &doc.syntax);
        let mut commands: BTreeMap<_, _> = doc
            .syntax
            .commands
            .iter()
            .filter(|command| positions.contains(&command.span.start))
            .map(|command| (command.span.start, command.clone()))
            .collect();
        for call in self
            .inputs
            .calls
            .get(uri)
            .into_iter()
            .flatten()
            .chain(self.inputs.loaders.get(uri).into_iter().flatten())
        {
            if call.certain {
                commands.insert(call.command.span.start, call.command.clone());
            } else {
                self.complete = false;
            }
        }
        let mut ignored = doc.syntax.ignored.clone();
        ignored.extend(definition_body_spans(&doc.text.source, &doc.syntax));
        ignored.extend(
            doc.syntax
                .commands
                .iter()
                .map(|command| command.span.clone()),
        );
        ignored.sort_by_key(|span| span.start);
        let mut raw = 0;
        let mut ignored_index = 0;
        let limit = doc
            .syntax
            .commands
            .iter()
            .find(|command| command.name == "endinput" && positions.contains(&command.span.start))
            .map_or(doc.text.source.len(), |command| {
                doc.text.source[command.span.end..]
                    .find(['\r', '\n'])
                    .map_or(doc.text.source.len(), |after| command.span.end + after)
            });
        for (offset, command) in commands {
            self.events += 1;
            if self.events > 65536 || self.groups.len() > 256 || !self.complete {
                self.complete = false;
                break;
            }
            while raw < offset {
                while ignored_index < ignored.len() && ignored[ignored_index].end <= raw {
                    ignored_index += 1;
                }
                if let Some(span) = ignored.get(ignored_index).filter(|span| span.start <= raw) {
                    raw = span.end;
                    continue;
                }
                match doc.text.source.as_bytes()[raw] {
                    b'{' if self.groups.len() < 256 => self.groups.push(self.state.clone()),
                    b'{' => {
                        self.complete = false;
                        break;
                    }
                    b'}' => self.state = self.groups.pop().unwrap_or(None),
                    _ => {}
                }
                raw += 1;
            }
            // Commands inside a consumed filename/options group are data.
            if offset < raw {
                continue;
            }
            raw = command.span.end;
            match command.name.as_str() {
                "DeclareGraphicsExtensions" => {
                    self.state = if self.conditional == 0 {
                        declaration(&doc.text.source, &command)
                    } else {
                        None
                    };
                    raw = command
                        .argument
                        .as_ref()
                        .map_or(raw, |argument| argument.outer_span.end);
                }
                "includegraphics" => {
                    let key = (uri.clone(), offset);
                    self.settings
                        .entry(key)
                        .and_modify(|previous| {
                            if *previous != self.state {
                                *previous = None;
                            }
                        })
                        .or_insert_with(|| self.state.clone());
                    raw = command
                        .argument
                        .as_ref()
                        .map_or(raw, |argument| argument.outer_span.end);
                }
                "begingroup" | "bgroup" | "begin" => {
                    if command.name == "begin" {
                        if let Some(argument) = command
                            .argument
                            .as_ref()
                            .filter(|argument| syntax::is_verbatim_environment(&argument.value))
                        {
                            raw = doc
                                .syntax
                                .ignored
                                .iter()
                                .find(|span| span.start == argument.outer_span.end)
                                .map_or(argument.outer_span.end, |span| span.end);
                            continue;
                        }
                    }
                    self.groups.push(self.state.clone());
                    if command.name == "begin" {
                        raw = command
                            .argument
                            .as_ref()
                            .map_or(raw, |argument| argument.outer_span.end);
                    }
                }
                "endgroup" | "egroup" | "end" => {
                    self.state = self.groups.pop().unwrap_or(None);
                    if command.name == "end" {
                        raw = command
                            .argument
                            .as_ref()
                            .map_or(raw, |argument| argument.outer_span.end);
                    }
                }
                "global" => {
                    self.state = None;
                    self.groups.fill(None);
                }
                name if name.starts_with("if") => {
                    self.conditional += 1;
                    self.state = None;
                }
                "else" | "or" => self.state = None,
                "fi" => {
                    self.conditional = self.conditional.saturating_sub(1);
                    self.state = None;
                }
                "IfFileExists" | "InputIfFileExists" | "subfile" | "includeonly" => {
                    self.state = None;
                    self.complete = false;
                }
                name if matches!(name, "input" | "include")
                    || syntax::loader_extension(name).is_some() =>
                {
                    if !self.signatures.command_unchanged(name) {
                        self.complete = false;
                        break;
                    }
                    let Some(argument) = syntax::file_argument(&doc.text.source, &command) else {
                        self.state = None;
                        continue;
                    };
                    let names = if syntax::file_list(name) {
                        syntax::complete_file_list_spans(&doc.text.source, &command).map(|spans| {
                            spans
                                .into_iter()
                                .map(|span| doc.text.source[span].to_owned())
                                .collect::<Vec<_>>()
                        })
                    } else {
                        Some(vec![argument.value.clone()])
                    };
                    let Some(names) = names else {
                        self.state = None;
                        continue;
                    };
                    let package = syntax::loader_extension(name);
                    for value in names {
                        if !syntax::is_literal_file_path(&value) {
                            self.state = None;
                            continue;
                        }
                        let target =
                            resolve_file_path(self.root, name, &value, &[package.unwrap_or("tex")])
                                .or_else(|| {
                                    resolve_file_path(
                                        uri,
                                        name,
                                        &value,
                                        &[package.unwrap_or("tex")],
                                    )
                                });
                        if let Some(target) = target {
                            if package.is_none() || self.packages.insert(target.clone()) {
                                self.source(&target);
                            }
                        } else if package.is_none() {
                            self.state = None;
                        }
                    }
                    raw = argument.outer_span.end;
                }
                _ => {}
            }
        }
        // Closing braces at the end of a file still restore caller state.
        while raw < limit && self.complete {
            while ignored_index < ignored.len() && ignored[ignored_index].end <= raw {
                ignored_index += 1;
            }
            if let Some(span) = ignored.get(ignored_index).filter(|span| span.start <= raw) {
                raw = span.end;
                continue;
            }
            match doc.text.source.as_bytes()[raw] {
                b'{' if self.groups.len() < 256 => self.groups.push(self.state.clone()),
                b'{' => {
                    self.complete = false;
                    break;
                }
                b'}' => self.state = self.groups.pop().unwrap_or(None),
                _ => {}
            }
            raw += 1;
        }
        self.active.remove(uri);
    }
}
