//! Ordered, project-specific meanings of etoolbox's literal string tests.
use crate::{
    signature::{literal_copy_target, CommandShape, CommandSignatures},
    string_tests::{arguments, data_count},
    syntax::{self, Command, Syntax},
    workspace::{resolve_file_path, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};
use tower_lsp::lsp_types::Url;

const ONE: u8 = 1;
const TWO: u8 = 2;
const CUSTOM: u8 = 4;
const UNKNOWN: u8 = ONE | TWO | CUSTOM;

fn native(count: usize) -> u8 {
    if count == 1 {
        ONE
    } else {
        TWO
    }
}

fn default_meaning(command: &Command, known: bool) -> u8 {
    data_count(&command.name)
        .filter(|_| known)
        .map_or(CUSTOM, native)
}

fn literal_prefix(meaning: u8) -> Option<usize> {
    if meaning & CUSTOM != 0 {
        None
    } else if meaning & ONE != 0 {
        Some(1)
    } else if meaning & TWO != 0 {
        Some(2)
    } else {
        None
    }
}

fn exact_count(meaning: u8) -> Option<usize> {
    match meaning {
        ONE => Some(1),
        TWO => Some(2),
        _ => None,
    }
}

fn ambiguous(meaning: u8) -> bool {
    meaning.count_ones() > 1
}

fn maximum_count(meaning: u8) -> usize {
    if meaning & TWO != 0 {
        2
    } else {
        1
    }
}
const MAX_VISITS: usize = 4096;

fn file_test(name: &str) -> bool {
    matches!(name, "InputIfFileExists" | "IfFileExists")
}

struct Conditional {
    hook: Range<usize>,
    input: Option<Url>,
    end: usize,
}

impl Conditional {
    fn new(source: &str, command: &Command, uri: &Url, root: &Url) -> Option<Self> {
        let argument = syntax::file_argument(source, command)?;
        if argument.outer_span.start != syntax::skip_space(source, command.span.end)
            || !syntax::is_literal_file_path(&argument.value)
        {
            return None;
        }
        let file = resolve_file_path(root, &command.name, &argument.value, &["tex"])
            .or_else(|| resolve_file_path(uri, &command.name, &argument.value, &["tex"]));
        // A missing unqualified name may still exist in the TeX distribution.
        // Explicit paths can be checked within the documented local lookup.
        if file.is_none() && !argument.value.contains('/') {
            return None;
        }
        let (found, at) = syntax::group(
            source,
            syntax::skip_space(source, argument.outer_span.end),
            b'{',
            b'}',
        )?;
        let (missing, end) = syntax::group(source, syntax::skip_space(source, at), b'{', b'}')?;
        let hook = if file.is_some() {
            found.span
        } else {
            missing.span
        };
        Some(Self {
            hook,
            input: (command.name == "InputIfFileExists")
                .then_some(file)
                .flatten(),
            end,
        })
    }
}

enum Event<'a> {
    Command(&'a Command),
    Input { uri: Url, at: usize },
}

#[derive(Clone, Default)]
enum Selection {
    #[default]
    All,
    Only(BTreeSet<String>),
    Unknown,
}

fn chapter_name(value: &str) -> &str {
    value.strip_suffix(".tex").unwrap_or(value)
}

#[derive(Clone)]
enum Binding {
    Native(usize),
    Custom(Option<Arc<CommandShape>>),
    Unknown,
}

#[derive(Clone, Default)]
struct Context {
    assigned: BTreeMap<String, Binding>,
    selection: Selection,
    uncertain: bool,
}

impl Context {
    fn binding(&self, name: &str) -> Binding {
        self.assigned
            .get(name)
            .cloned()
            .unwrap_or_else(|| data_count(name).map_or(Binding::Custom(None), Binding::Native))
    }

    fn meaning(&self, name: &str) -> u8 {
        if self.uncertain {
            return UNKNOWN;
        }
        match self.binding(name) {
            Binding::Native(count) => native(count),
            Binding::Custom(_) => CUSTOM,
            Binding::Unknown => UNKNOWN,
        }
    }

    fn assign(&mut self, command: &Command, declaration: &Assignment) {
        let binding = if declaration.copy {
            if self.uncertain || self.assigned.contains_key(&command.name) {
                Binding::Unknown
            } else {
                declaration
                    .source
                    .as_deref()
                    .map_or(Binding::Unknown, |name| self.binding(name))
            }
        } else {
            Binding::Custom(declaration.shape.clone())
        };
        self.assigned.insert(declaration.name.clone(), binding);
    }
}

struct Assignment {
    name: String,
    source: Option<String>,
    copy: bool,
    shape: Option<Arc<CommandShape>>,
}

struct Source {
    syntax: Syntax,
    active: BTreeSet<usize>,
    operands: BTreeSet<usize>,
    lexical_probes: Vec<Command>,
    declarations: BTreeMap<usize, Vec<Assignment>>,
}

impl Source {
    fn new(
        text: &crate::text::Text,
        mut literal: impl FnMut(&crate::syntax::Command, bool) -> u8,
        shapes: &BTreeMap<usize, Arc<CommandShape>>,
    ) -> Self {
        let mut uncertain = BTreeSet::new();
        let mut lexical_probes = Vec::new();
        let mut syntax = Syntax::latex_with_string_tests(text, |command, known| {
            // The primitive verb is omitted from the normal command index.
            // Keep its invocation visible to the ordered analysis: a prior
            // assignment may have replaced their delimiter-based grammar.
            if command.name == "verb" {
                lexical_probes.push(command.clone());
            }
            let meaning = literal(command, known);
            if ambiguous(meaning) {
                uncertain.insert(command.span.start);
            }
            literal_prefix(meaning)
        });
        lexical_probes.retain(|probe| {
            !syntax
                .commands
                .iter()
                .any(|command| command.span.start == probe.span.start)
        });
        for at in uncertain {
            syntax.string_test_arities.insert(at, None);
        }
        syntax.contextual_command_shapes = shapes.clone();
        let signatures = CommandSignatures::source_context([(&*text.source, &syntax)]);
        let active = signatures.source_command_positions(&text.source, &syntax);
        let operands = signatures.known_argument_tokens(&text.source, &syntax);
        let mut declarations: BTreeMap<usize, Vec<Assignment>> = BTreeMap::new();
        for entry in &syntax.definitions {
            let Some(name) = entry.detail.strip_prefix("Defined with \\") else {
                continue;
            };
            let before = syntax
                .commands
                .partition_point(|command| command.span.start < entry.span.start);
            if let Some(command) = syntax.commands[..before]
                .iter()
                .rev()
                .find(|command| command.name == name)
            {
                declarations
                    .entry(command.span.start)
                    .or_default()
                    .push(Assignment {
                        name: entry.name.clone(),
                        source: literal_copy_target(&text.source, &syntax, entry, false)
                            .map(|(name, _)| name),
                        copy: matches!(name, "let" | "cslet" | "letcs" | "csletcs")
                            || syntax::declares_command_copy(name),
                        shape: CommandShape::definition(&text.source, &syntax, entry).map(Arc::new),
                    });
            }
        }
        Self {
            syntax,
            active,
            operands,
            lexical_probes,
            declarations,
        }
    }
}

#[derive(Clone, Default)]
struct Analysis {
    meanings: BTreeMap<Url, BTreeMap<usize, u8>>,
    operands: BTreeMap<Url, BTreeMap<usize, bool>>,
    names: BTreeSet<String>,
    shapes: BTreeMap<Url, BTreeMap<usize, Option<Arc<CommandShape>>>>,
    visited: BTreeSet<Url>,
    missing: BTreeMap<Url, DocumentKind>,
    excluded: BTreeSet<Url>,
    visits: usize,
    incomplete: bool,
}

impl Analysis {
    fn walk(
        &mut self,
        workspace: &Workspace,
        uri: &Url,
        root: &Url,
        state: &mut Context,
        stack: &mut BTreeSet<Url>,
    ) {
        if stack.len() >= 16 || self.visits >= MAX_VISITS || !stack.insert(uri.clone()) {
            self.incomplete = true;
            return;
        }
        let Some(doc) = workspace
            .documents
            .get(uri)
            .filter(|doc| doc.kind == DocumentKind::Latex)
        else {
            stack.remove(uri);
            return;
        };
        let initial = state.clone();
        let checkpoint = self.clone();
        let mut context = BTreeMap::<usize, u8>::new();
        let mut shapes = BTreeMap::new();
        for round in 0..32 {
            // Start from literal grammar. Eagerly exposing every data operand
            // could let a quoted verbatim opener swallow the rest of the file.
            let source = Source::new(
                &doc.text,
                |command, known| {
                    context
                        .get(&command.span.start)
                        .copied()
                        .unwrap_or_else(|| {
                            if known && self.names.contains(&command.name) {
                                initial.meaning(&command.name)
                            } else {
                                default_meaning(command, known)
                            }
                        })
                },
                &shapes,
            );
            let visits = self.visits + 1;
            *self = checkpoint.clone();
            self.visits = visits;
            self.visited.insert(uri.clone());
            *state = initial.clone();
            let mut data_end = 0;
            let mut meanings = BTreeMap::new();
            let mut consumed = BTreeSet::new();
            let mut next_shapes = BTreeMap::new();
            let mut reparse = false;
            let mut events: Vec<_> = source
                .syntax
                .commands
                .iter()
                .chain(&source.lexical_probes)
                .map(Event::Command)
                .collect();
            events.sort_by_key(|event| {
                std::cmp::Reverse(match event {
                    Event::Command(command) => command.span.start,
                    Event::Input { at, .. } => *at,
                })
            });
            while let Some(event) = events.pop() {
                let command = match event {
                    Event::Command(command) => command,
                    Event::Input {
                        uri: dependency,
                        at,
                    } => {
                        if at >= data_end {
                            if workspace.documents.contains_key(&dependency) {
                                self.walk(workspace, &dependency, root, state, stack);
                            } else {
                                self.missing.insert(dependency, DocumentKind::Latex);
                            }
                        }
                        continue;
                    }
                };
                let at = command.span.start;
                if at < data_end || source.syntax.control_operands.contains(&at) {
                    continue;
                }
                if source.operands.contains(&at) {
                    // A control consumed by another command is a token, even
                    // if its saved meaning would normally read string data.
                    // Reparse to expose any following call swallowed by that
                    // token's former data grammar.
                    if self.names.contains(&command.name) {
                        consumed.insert(at);
                        meanings.insert(at, CUSTOM);
                        reparse |= source.syntax.string_test_arities.contains_key(&at);
                    }
                    continue;
                }
                let binding = state.binding(&command.name);
                if self.names.contains(&command.name)
                    || matches!(binding, Binding::Native(_) | Binding::Custom(Some(_)))
                {
                    // A copy keeps its captured data arity even when its source
                    // or its destination has another builtin spelling.
                    let meaning = state.meaning(&command.name);
                    if meaning == CUSTOM {
                        if let Binding::Custom(Some(shape)) = &binding {
                            next_shapes.insert(at, shape.clone());
                        }
                    }
                    meanings.insert(at, meaning);
                    let expected = if ambiguous(meaning) {
                        Some(None)
                    } else {
                        exact_count(meaning).map(Some)
                    };
                    reparse |= source.syntax.string_test_arities.get(&at).copied() != expected;
                    if let Some(count) = exact_count(meaning) {
                        self.names.insert(command.name.clone());
                        let data = arguments(&doc.text.source, command.span.end, count);
                        data_end = data.last().map_or(data_end, |argument| argument.end);
                        // A native string copy cannot perform a file load or
                        // selector operation merely because of its spelling.
                        continue;
                    }
                }
                if !source.active.contains(&at) {
                    continue;
                }
                if let Some(target) = source.syntax.command_undefinitions.get(&at) {
                    if let Some(target) = target {
                        let binding =
                            if state.uncertain || state.assigned.contains_key(&command.name) {
                                Binding::Unknown
                            } else {
                                Binding::Custom(Some(Arc::new(CommandShape::no_arguments())))
                            };
                        state.assigned.insert(target.name.clone(), binding);
                    } else {
                        // An expanded name can remove any tracked binding.
                        state.uncertain = true;
                    }
                }
                if let Some(declarations) = source.declarations.get(&at) {
                    for declaration in declarations {
                        state.assign(command, declaration);
                        if matches!(state.binding(&declaration.name), Binding::Native(_)) {
                            self.names.insert(declaration.name.clone());
                        }
                    }
                }
                if command.name == "includeonly" {
                    state.selection = if state.assigned.contains_key("includeonly") {
                        Selection::Unknown
                    } else if let Some(names) = syntax::includeonly_names(&doc.text.source, command)
                    {
                        Selection::Only(names.into_iter().collect())
                    } else {
                        Selection::Unknown
                    };
                    continue;
                }
                if command.name == "include" {
                    let selected = if state.assigned.contains_key("include") {
                        None
                    } else {
                        match &state.selection {
                            Selection::All => Some(true),
                            Selection::Only(names) => {
                                syntax::file_argument(&doc.text.source, command)
                                    .filter(|argument| {
                                        syntax::is_literal_file_path(&argument.value)
                                    })
                                    .map(|argument| names.contains(chapter_name(&argument.value)))
                            }
                            Selection::Unknown => None,
                        }
                    };
                    if selected == Some(false) {
                        // Keep excluded chapters available to the editor, but
                        // do not execute their dependency trees as new roots.
                        for (dependency, _) in
                            workspace.literal_command_dependencies(uri, root, command)
                        {
                            self.excluded
                                .extend(workspace.project_documents(&dependency));
                        }
                        continue;
                    }
                    state.uncertain |= selected.is_none();
                }
                if file_test(&command.name) {
                    if let Some(conditional) = (!state.assigned.contains_key(&command.name))
                        .then(|| Conditional::new(&doc.text.source, command, uri, root))
                        .flatten()
                    {
                        // Hooks are token arguments. Execute only the selected
                        // hook, and schedule the input after its commands.
                        while matches!(events.last(), Some(Event::Command(next)) if next.span.start < conditional.end)
                        {
                            events.pop();
                        }
                        if let Some(dependency) = conditional.input {
                            events.push(Event::Input {
                                uri: dependency,
                                at: conditional.end,
                            });
                        }
                        events.extend(
                            source
                                .syntax
                                .commands
                                .iter()
                                .rev()
                                .filter(|next| conditional.hook.contains(&next.span.start))
                                .map(Event::Command),
                        );
                    } else {
                        self.incomplete = true;
                    }
                    continue;
                }
                for (dependency, kind) in workspace.literal_command_dependencies(uri, root, command)
                {
                    if kind != DocumentKind::Latex {
                        continue;
                    }
                    if !workspace.documents.contains_key(&dependency) {
                        self.missing.insert(dependency, kind);
                    } else {
                        self.walk(workspace, &dependency, root, state, stack);
                    }
                }
            }
            reparse |= shapes != next_shapes;
            if reparse && round < 31 && self.visits < MAX_VISITS {
                context = meanings;
                shapes = next_shapes;
                continue;
            }
            self.incomplete |= reparse;
            for (at, meaning) in meanings {
                let operand = consumed.contains(&at);
                self.operands
                    .entry(uri.clone())
                    .or_default()
                    .entry(at)
                    .and_modify(|previous| *previous &= operand)
                    .or_insert(operand);
                if meaning == CUSTOM {
                    let shape = next_shapes.get(&at).cloned();
                    self.shapes
                        .entry(uri.clone())
                        .or_default()
                        .entry(at)
                        .and_modify(|previous| {
                            if *previous != shape {
                                *previous = None;
                            }
                        })
                        .or_insert(shape);
                }
                *self
                    .meanings
                    .entry(uri.clone())
                    .or_default()
                    .entry(at)
                    .or_default() |= meaning;
            }
            break;
        }
        stack.remove(uri);
    }
}

pub(crate) fn refresh(workspace: &mut Workspace) {
    let mut names: BTreeSet<String> = ["ifstrequal", "ifstrempty", "ifblank", "notblank"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    // Copy destinations are potential string entries in every root, including
    // roots which give the same spelling a custom meaning.
    for _ in 0..32 {
        let before = names.len();
        for doc in workspace
            .documents
            .values()
            .filter(|doc| doc.kind == DocumentKind::Latex)
        {
            for entry in &doc.syntax.definitions {
                if literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                    .is_some_and(|(source, _)| names.contains(&source))
                {
                    names.insert(entry.name.clone());
                }
            }
        }
        if names.len() == before {
            break;
        }
    }
    let native_operand = || {
        let documents = || {
            workspace
                .documents
                .values()
                .filter(|doc| doc.kind == DocumentKind::Latex)
        };
        // Preserve the need for analysis after an earlier pass removed a
        // predicate's native arity at a confirmed operand position.
        if documents().any(|doc| doc.syntax.contextual_string_operands) {
            return true;
        }
        if !documents().any(|doc| !doc.syntax.string_test_arities.is_empty()) {
            return false;
        }
        let signatures = CommandSignatures::source_context(
            documents().map(|doc| (doc.text.source.as_str(), &doc.syntax)),
        );
        documents().any(|doc| {
            doc.syntax.string_test_arities.keys().any(|&at| {
                // A possible grammar schedules analysis; each root subsequently
                // confirms its own shapes in execution order.
                signatures.declared_literal_is_argument(&doc.text.source, &doc.syntax, at)
            })
        })
    };
    if names.len() == 4
        && !workspace
            .documents
            .values()
            .filter(|doc| doc.kind == DocumentKind::Latex)
            .any(|doc| {
                !doc.syntax.command_undefinitions.is_empty()
                    || doc
                        .syntax
                        .definitions
                        .iter()
                        .any(|entry| data_count(&entry.name).is_some())
            })
        && !native_operand()
    {
        for doc in workspace
            .documents
            .values_mut()
            .filter(|doc| doc.syntax.contextual_string_tests)
        {
            doc.syntax = Syntax::latex(&doc.text);
        }
        return;
    }
    // A custom call can expose another input previously hidden as data.
    for round in 0..16 {
        let sources: BTreeMap<_, _> = workspace
            .documents
            .iter()
            .filter(|(_, doc)| doc.kind == DocumentKind::Latex)
            .map(|(uri, doc)| {
                (
                    uri.clone(),
                    Source::new(&doc.text, default_meaning, &BTreeMap::new()),
                )
            })
            .collect();
        let mut analysis = Analysis {
            names: names.clone(),
            ..Default::default()
        };
        let roots: Vec<_> = sources
            .keys()
            .filter(|uri| workspace.is_saved_root(uri))
            .cloned()
            .collect();
        for root in &roots {
            analysis.walk(
                workspace,
                root,
                root,
                &mut Context::default(),
                &mut BTreeSet::new(),
            );
        }
        // Prefer graph entries so a child does not add a false empty context.
        let children: BTreeSet<_> = sources
            .iter()
            .flat_map(|(uri, source)| {
                source
                    .syntax
                    .commands
                    .iter()
                    .filter(|command| source.active.contains(&command.span.start))
                    .flat_map(|command| workspace.literal_command_dependencies(uri, uri, command))
                    .map(|(uri, _)| uri)
                    .collect::<Vec<_>>()
            })
            .collect();
        for uri in sources.keys().filter(|uri| !children.contains(*uri)) {
            if !analysis.visited.contains(uri) && !analysis.excluded.contains(uri) {
                analysis.walk(
                    workspace,
                    uri,
                    uri,
                    &mut Context::default(),
                    &mut BTreeSet::new(),
                );
            }
        }
        for uri in sources.keys() {
            if !analysis.visited.contains(uri) && !analysis.excluded.contains(uri) {
                analysis.walk(
                    workspace,
                    uri,
                    uri,
                    &mut Context::default(),
                    &mut BTreeSet::new(),
                );
            }
        }
        if analysis.names != names {
            if round < 15 {
                names = analysis.names.clone();
                continue;
            }
            analysis.incomplete = true;
        }
        if !analysis.missing.is_empty() && round < 15 {
            let before = workspace.documents.len();
            for (uri, kind) in &analysis.missing {
                workspace.reload_document_as(uri, true, Some(*kind));
            }
            if workspace.documents.len() > before {
                continue;
            }
        }
        analysis.incomplete |= !analysis.missing.is_empty();
        for (uri, source) in sources {
            let doc = workspace.documents.get_mut(&uri).unwrap();
            let meanings = analysis.meanings.get(&uri);
            let mut syntax = Syntax::latex_with_string_tests(&doc.text, |command, known| {
                if analysis.incomplete {
                    return None;
                }
                meanings
                    .and_then(|meanings| meanings.get(&command.span.start))
                    .map_or_else(
                        || literal_prefix(default_meaning(command, known)),
                        |meaning| literal_prefix(*meaning),
                    )
            });
            if let Some(meanings) = meanings {
                for (&at, &meaning) in meanings {
                    if !ambiguous(meaning) && !analysis.incomplete {
                        continue;
                    }
                    syntax.string_test_arities.insert(at, None);
                    let Some(word) = crate::syntax::control_word(&doc.text.source, at) else {
                        continue;
                    };
                    let common = if analysis.incomplete {
                        0
                    } else {
                        literal_prefix(meaning).unwrap_or(0)
                    };
                    syntax.ambiguous_string_tests.extend(
                        arguments(&doc.text.source, word.end, maximum_count(meaning))
                            .into_iter()
                            .skip(common)
                            .map(|argument| argument.span),
                    );
                }
            }
            if analysis.incomplete {
                for command in &source.syntax.commands {
                    if analysis.names.contains(&command.name) {
                        syntax.string_test_arities.insert(command.span.start, None);
                        syntax.ambiguous_string_tests.extend(
                            arguments(&doc.text.source, command.span.end, 2)
                                .into_iter()
                                .map(|argument| argument.span),
                        );
                    }
                }
            }
            if !analysis.incomplete {
                if let (Some(meanings), Some(shapes)) = (meanings, analysis.shapes.get(&uri)) {
                    for (&at, shape) in shapes {
                        if meanings.get(&at) == Some(&CUSTOM) {
                            if let Some(shape) = shape {
                                syntax.contextual_command_shapes.insert(at, shape.clone());
                            }
                        }
                    }
                }
            }
            if !analysis.incomplete {
                if let Some(operands) = analysis.operands.get(&uri) {
                    syntax.contextual_string_operands = operands.values().any(|all| *all);
                    syntax
                        .control_operands
                        .extend(operands.iter().filter_map(|(&at, &all)| all.then_some(at)));
                }
            }
            syntax.contextual_string_tests = true;
            doc.syntax = syntax;
        }
        break;
    }
}
