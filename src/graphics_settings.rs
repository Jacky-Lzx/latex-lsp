//! Ordered graphicx settings, shared by navigation, editing and build checks.
use crate::{
    signature::{definition_body_spans, CommandSignatures},
    source_inputs, syntax,
    workspace::{resolve_file_path, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

type Setting = Option<Vec<String>>;

#[derive(Clone, Copy)]
enum Kind {
    Extensions,
    Directories,
}

impl Kind {
    fn command(self) -> &'static str {
        match self {
            Self::Extensions => "DeclareGraphicsExtensions",
            Self::Directories => "graphicspath",
        }
    }
}

#[derive(Clone)]
struct Group {
    setting: Setting,
    global_defs: i8,
}

const SETTING_CONTROLS: &[&str] = &[
    "global",
    "globaldefs",
    "long",
    "outer",
    "protected",
    "DeclareGraphicsExtensions",
    "graphicspath",
];

fn setting_call(
    source: &str,
    commands: &[syntax::Command],
    index: usize,
    signatures: &CommandSignatures<'_>,
) -> Option<(&'static str, usize)> {
    let command = &commands[index];
    let (name, end) = if command.name == "csuse" {
        let argument = command.argument.as_ref()?;
        (argument.value.as_str(), argument.outer_span.end)
    } else if command.name == "csname" {
        let span = syntax::literal_csname(source, commands, index)?;
        (&source[span], commands[index + 1].span.end)
    } else {
        (command.name.as_str(), command.span.end)
    };
    SETTING_CONTROLS
        .iter()
        .find(|builtin| signatures.invokes_builtin(name, builtin))
        .map(|builtin| (*builtin, end))
}

fn named_graphics_command(
    source: &str,
    commands: &[syntax::Command],
    quoted: &BTreeSet<usize>,
    names: &[&str],
) -> bool {
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
        name.is_some_and(|name| names.contains(&name))
    })
}

pub(crate) struct Index {
    defaults: Vec<String>,
    settings: Option<BTreeMap<(Url, usize), Setting>>,
}

impl Index {
    pub fn new(workspace: &Workspace, root: &Url) -> Self {
        Self::build(workspace, root, Kind::Extensions)
    }

    pub fn directories(workspace: &Workspace, root: &Url) -> Self {
        Self::build(workspace, root, Kind::Directories)
    }

    fn build(workspace: &Workspace, root: &Url, kind: Kind) -> Self {
        let defaults = match kind {
            Kind::Extensions => syntax::file_extensions("includegraphics")
                .unwrap()
                .iter()
                .map(|extension| (*extension).to_owned())
                .collect(),
            Kind::Directories => Vec::new(),
        };
        let customized_commands = match kind {
            Kind::Extensions => vec!["DeclareGraphicsExtensions", "DeclareGraphicsRule"],
            Kind::Directories => vec!["graphicspath"],
        };
        let scope = workspace.project_documents(root);
        let documents: Vec<_> = scope
            .iter()
            .filter_map(|uri| workspace.documents.get(uri).map(|doc| (uri, doc)))
            .filter(|(_, doc)| doc.kind == DocumentKind::Latex)
            .collect();
        let customized = documents.iter().any(|(_, doc)| {
            doc.syntax
                .commands
                .iter()
                .any(|command| customized_commands.contains(&command.name.as_str()))
                || doc
                    .syntax
                    .definitions
                    .iter()
                    .any(|entry| customized_commands.contains(&entry.name.as_str()))
                || named_graphics_command(
                    &doc.text.source,
                    &doc.syntax.commands,
                    &BTreeSet::new(),
                    &customized_commands,
                )
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
            "graphicspath",
            "DeclareGraphicsRule",
            "includegraphics",
            "begingroup",
            "endgroup",
            "bgroup",
            "egroup",
            "begin",
            "end",
            "global",
            "globaldefs",
            "long",
            "outer",
            "protected",
            "csuse",
            "csname",
            "endcsname",
        ]
        .iter()
        .any(|name| !signatures.command_unchanged(name))
            || documents.iter().any(|(_, doc)| {
                let positions = signatures.source_command_positions(&doc.text.source, &doc.syntax);
                let quoted = signatures.quoted_command_tokens(&doc.text.source, &doc.syntax);
                let alias_bodies = signatures.builtin_alias_body_positions(
                    &doc.text.source,
                    &doc.syntax,
                    SETTING_CONTROLS,
                );
                let declaration_targets: BTreeSet<_> = doc
                    .syntax
                    .definitions
                    .iter()
                    .map(|entry| entry.span.start)
                    .collect();
                let numeric_operands: Vec<_> = doc
                    .syntax
                    .commands
                    .iter()
                    .enumerate()
                    .filter(|(_, command)| positions.contains(&command.span.start))
                    .filter_map(|(index, _)| {
                        let (name, end) = setting_call(
                            &doc.text.source,
                            &doc.syntax.commands,
                            index,
                            &signatures,
                        )?;
                        (name == "globaldefs")
                            .then(|| {
                                signatures
                                    .literal_integer_value(&doc.text.source, end)
                                    .map(|(_, after)| end..after)
                            })
                            .flatten()
                    })
                    .collect();
                named_graphics_command(
                    &doc.text.source,
                    &doc.syntax.commands,
                    &quoted,
                    &["DeclareGraphicsRule"],
                ) || doc
                    .syntax
                    .commands
                    .iter()
                    .enumerate()
                    .any(|(index, command)| {
                        !quoted.contains(&command.span.start)
                            && (command.name == "DeclareGraphicsRule"
                                || positions.contains(&command.span.start)
                                    && (command.name == "csuse"
                                        && command.argument.as_ref().is_none_or(|argument| {
                                            !syntax::is_static_key(&argument.value)
                                        })
                                        || command.name == "csname"
                                            && syntax::literal_csname(
                                                &doc.text.source,
                                                &doc.syntax.commands,
                                                index,
                                            )
                                            .is_none())
                                || setting_call(
                                    &doc.text.source,
                                    &doc.syntax.commands,
                                    index,
                                    &signatures,
                                )
                                .is_some()
                                    && !positions.contains(&command.span.start)
                                    && !declaration_targets.contains(&command.span.start)
                                    && !alias_bodies.contains(&command.span.start)
                                    && !numeric_operands
                                        .iter()
                                        .any(|span| span.contains(&command.span.start)))
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
            kind,
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
            global_defs: 0,
            global_prefix: false,
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
                !doc.syntax
                    .commands
                    .iter()
                    .enumerate()
                    .any(|(index, command)| {
                        !quoted.contains(&command.span.start)
                            && setting_call(
                                &doc.text.source,
                                &doc.syntax.commands,
                                index,
                                &walk.signatures,
                            )
                            .is_some_and(|(name, _)| name == kind.command())
                    })
            }
        });
        if walk.complete && ordered {
            index.settings = Some(walk.settings);
        }
        index
    }
    /// Keep lexical aliases for build snapshots; navigation and edits use canonical directories.
    pub fn paths_at(
        &self,
        root: &Url,
        uri: &Url,
        offset: usize,
        canonical: bool,
    ) -> Option<Vec<std::path::PathBuf>> {
        let root_path = root.to_file_path().ok()?;
        let parent = root_path.parent()?;
        Some(
            self.at(uri, offset)?
                .into_iter()
                .map(|value| {
                    let path = parent.join(value);
                    if canonical {
                        path.canonicalize()
                            .unwrap_or_else(|_| crate::workspace::normalize_path(&path))
                    } else {
                        path
                    }
                })
                .collect(),
        )
    }

    pub fn at(&self, uri: &Url, offset: usize) -> Option<Vec<&str>> {
        let extensions = match &self.settings {
            None => &self.defaults,
            Some(settings) => settings.get(&(uri.clone(), offset))?.as_ref()?,
        };
        Some(extensions.iter().map(String::as_str).collect())
    }
}

fn declaration(
    source: &str,
    command: &syntax::Command,
    kind: Kind,
    quoted_directories: bool,
) -> Setting {
    if matches!(kind, Kind::Directories) {
        let (entries, complete) =
            crate::graphics::path_entries(source, command, quoted_directories);
        return complete.then(|| entries.into_iter().map(|(_, value)| value).collect());
    }
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
    kind: Kind,
    workspace: &'a Workspace,
    root: &'a Url,
    signatures: CommandSignatures<'a>,
    inputs: source_inputs::Index,
    settings: BTreeMap<(Url, usize), Setting>,
    active: BTreeSet<Url>,
    visited: BTreeSet<Url>,
    packages: BTreeSet<Url>,
    state: Setting,
    groups: Vec<Group>,
    global_defs: i8,
    global_prefix: bool,
    conditional: usize,
    visits: usize,
    events: usize,
    bytes: usize,
    complete: bool,
}

impl Walk<'_> {
    fn open_group(&mut self) {
        self.groups.push(Group {
            setting: self.state.clone(),
            global_defs: self.global_defs,
        });
    }

    fn close_group(&mut self) {
        if let Some(group) = self.groups.pop() {
            self.state = group.setting;
            self.global_defs = group.global_defs;
        } else {
            self.complete = false;
        }
    }

    fn global_assignment(&mut self) -> bool {
        let global = self.global_defs > 0 || self.global_defs == 0 && self.global_prefix;
        self.global_prefix = false;
        global
    }

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
        let images =
            self.signatures
                .graphics_invocations(&doc.text.source, &doc.syntax, &positions);
        for call in self
            .inputs
            .calls
            .get(uri)
            .into_iter()
            .flatten()
            .chain(self.inputs.loaders.get(uri).into_iter().flatten())
            .chain(&images)
        {
            if call.certain {
                commands.insert(call.command.span.start, call.command.clone());
            } else {
                self.complete = false;
            }
        }
        // Settings and assignment prefixes may be reached through pure
        // aliases or static named calls; their operands start after that call.
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if !positions.contains(&command.span.start) {
                continue;
            }
            if let Some((name, end)) = setting_call(
                &doc.text.source,
                &doc.syntax.commands,
                index,
                &self.signatures,
            ) {
                let mut normalized = command.clone();
                normalized.name = name.to_owned();
                normalized.span.end = end;
                if matches!(name, "DeclareGraphicsExtensions" | "graphicspath") {
                    normalized.argument = syntax::group(
                        &doc.text.source,
                        syntax::skip_space(&doc.text.source, end),
                        b'{',
                        b'}',
                    )
                    .map(|(argument, _)| argument);
                }
                commands.insert(command.span.start, normalized);
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
                    b'{' if self.groups.len() < 256 => self.open_group(),
                    b'{' => {
                        self.complete = false;
                        break;
                    }
                    b'}' => self.close_group(),
                    _ => {}
                }
                raw += 1;
            }
            // Commands inside a consumed filename/options group are data.
            if offset < raw {
                continue;
            }
            raw = command.span.end;
            if self.global_prefix
                && !matches!(
                    command.name.as_str(),
                    "global"
                        | "globaldefs"
                        | "long"
                        | "outer"
                        | "protected"
                        | "DeclareGraphicsExtensions"
                        | "graphicspath"
                        | "input"
                )
            {
                // Other literal primitive assignments consume the prefix
                // without changing graphics settings. Unknown expansion does
                // not establish which assignment will receive it.
                if matches!(
                    command.name.as_str(),
                    "def"
                        | "gdef"
                        | "edef"
                        | "xdef"
                        | "let"
                        | "futurelet"
                        | "count"
                        | "dimen"
                        | "skip"
                        | "muskip"
                        | "toks"
                        | "advance"
                        | "multiply"
                        | "divide"
                        | "chardef"
                        | "mathchardef"
                        | "countdef"
                        | "dimendef"
                        | "skipdef"
                        | "muskipdef"
                        | "toksdef"
                ) {
                    if !self.signatures.command_unchanged(&command.name) {
                        self.complete = false;
                        break;
                    }
                    self.global_prefix = false;
                } else {
                    self.complete = false;
                    break;
                }
            }
            match command.name.as_str() {
                "DeclareGraphicsExtensions" | "graphicspath" => {
                    let global = self.global_assignment();
                    if command.name == self.kind.command() {
                        self.state = if self.conditional == 0 {
                            declaration(
                                &doc.text.source,
                                &command,
                                self.kind,
                                self.workspace.build_config(self.root).effective_engine()
                                    == Some(crate::config::Engine::PdfLatex),
                            )
                        } else {
                            None
                        };
                        if global {
                            for group in &mut self.groups {
                                group.setting = self.state.clone();
                            }
                        }
                    }
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
                    self.open_group();
                    if command.name == "begin" {
                        raw = command
                            .argument
                            .as_ref()
                            .map_or(raw, |argument| argument.outer_span.end);
                    }
                }
                "endgroup" | "egroup" | "end" => {
                    self.close_group();
                    if command.name == "end" {
                        raw = command
                            .argument
                            .as_ref()
                            .map_or(raw, |argument| argument.outer_span.end);
                    }
                }
                "global" => self.global_prefix = true,
                "long" | "outer" | "protected" => {}
                "globaldefs" => {
                    if self.conditional != 0 {
                        self.complete = false;
                        break;
                    }
                    let Some((value, end)) = self
                        .signatures
                        .literal_integer_value(&doc.text.source, command.span.end)
                    else {
                        self.complete = false;
                        break;
                    };
                    let global = self.global_assignment();
                    self.global_defs = value.signum() as i8;
                    if global {
                        for group in &mut self.groups {
                            group.global_defs = self.global_defs;
                        }
                    }
                    raw = end;
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
                b'{' if self.groups.len() < 256 => self.open_group(),
                b'{' => {
                    self.complete = false;
                    break;
                }
                b'}' => self.close_group(),
                _ => {}
            }
            raw += 1;
        }
        self.active.remove(uri);
    }
}
