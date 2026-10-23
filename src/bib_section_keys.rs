//! Read-only citation provenance for statically ordered reference sections.
//!
//! This deliberately does not certify references or renames. Those operations
//! also need the identity of every declaration and bibliography relation.
use crate::{
    bib_section_files::Registry,
    named_references::Context,
    signature::CommandSignatures,
    syntax::{self, Command},
    workspace::{Document, DocumentKind, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind, Url};

const STATE: &[&str] = &[
    "newrefsection",
    "refsection",
    "endrefsection",
    "begin",
    "end",
    "addbibresource",
    "addglobalbib",
    "addsectionbib",
];
fn loader(name: &str) -> bool {
    matches!(name, "input" | "include" | "subfile") || syntax::loader_extension(name).is_some()
}
fn unsafe_control(name: &str) -> bool {
    name.starts_with("if")
        || name.starts_with("every")
        || matches!(
            name,
            "else"
                | "or"
                | "fi"
                | "unless"
                | "loop"
                | "repeat"
                | "whiledo"
                | "foreach"
                | "expandafter"
                | "noexpand"
                | "expanded"
                | "unexpanded"
                | "aftergroup"
                | "afterassignment"
                | "futurelet"
                | "let"
                | "edef"
                | "xdef"
                | "csedef"
                | "csxdef"
                | "cslet"
                | "csletcs"
                | "csundef"
                | "undef"
                | "catcode"
                | "makeatletter"
                | "globaldefs"
                | "count"
                | "countdef"
                | "advance"
                | "multiply"
                | "divide"
                | "scantokens"
                | "endlinechar"
                | "escapechar"
                | "ExplSyntaxOn"
                | "includeonly"
                | "InputIfFileExists"
                | "IfFileExists"
                | "import"
                | "subimport"
                | "bibliography"
                | "nobibliography"
        )
}

#[derive(Clone, PartialEq)]
struct Query {
    span: Range<usize>,
    command: usize,
}
fn query(doc: &Document, signatures: &CommandSignatures<'_>, offset: usize) -> Option<Query> {
    let context = Context::new(doc, signatures);
    let source = &doc.text.source;
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        if context.operand(doc, lexical.span.start) || context.literal(lexical.span.start) {
            continue;
        }
        let (name, span) = signatures
            .named_invocation(
                source,
                &doc.syntax,
                index,
                &BTreeSet::new(),
                &context.declarations,
            )
            .unwrap_or((&lexical.name, lexical.span.clone()));
        // Forwarded citation names require load-position meaning snapshots.
        if !syntax::is_citation(name) || !signatures.command_unchanged(name) {
            continue;
        }
        let command = crate::reference_calls::resolved_command(source, name, span);
        for (citation, span) in crate::reference_calls::keys(source, &command, false) {
            if citation && span.start <= offset && offset <= span.end {
                return Some(Query {
                    span,
                    command: lexical.span.start,
                });
            }
        }
    }
    None
}

fn citation_candidate(doc: &Document, signatures: &CommandSignatures<'_>, offset: usize) -> bool {
    let context = Context::new(doc, signatures);
    doc.syntax
        .commands
        .iter()
        .enumerate()
        .any(|(index, lexical)| {
            if context.literal(lexical.span.start) {
                return false;
            }
            let command = crate::reference_calls::named_command(
                &doc.text.source,
                &doc.syntax,
                index,
                signatures,
                &BTreeSet::new(),
                &context.declarations,
            )
            .or_else(|| {
                syntax::is_citation(&lexical.name).then(|| {
                    crate::reference_calls::resolved_command(
                        &doc.text.source,
                        &lexical.name,
                        lexical.span.clone(),
                    )
                })
            });
            command.is_some_and(|command| {
                crate::reference_calls::keys(&doc.text.source, &command, false)
                    .iter()
                    .any(|(citation, span)| *citation && span.start <= offset && offset <= span.end)
            })
        })
}

struct Analysis<'a, 'b> {
    ws: &'a Workspace,
    root: &'a Url,
    signatures: &'b CommandSignatures<'a>,
    registry: Registry,
    origin: &'a Url,
    query: Query,
    active: BTreeSet<Url>,
    answer: Option<BTreeSet<Url>>,
    visited: BTreeSet<Url>,
    stack: BTreeSet<Url>,
    visits: usize,
    commands: usize,
    work: usize,
    document: bool,
    ended: bool,
}
impl Analysis<'_, '_> {
    fn walk(&mut self, uri: &Url) -> Option<()> {
        self.visits += 1;
        if self.visits > 1024 || self.stack.len() >= 32 || !self.stack.insert(uri.clone()) {
            return None;
        }
        let doc = self
            .ws
            .documents
            .get(uri)
            .filter(|doc| doc.kind == DocumentKind::Latex)?;
        self.visited.insert(uri.clone());
        let source = &doc.text.source;
        let context = Context::new(doc, self.signatures);
        let positions = self
            .signatures
            .source_command_positions(source, &doc.syntax);
        for (index, lexical) in doc.syntax.commands.iter().enumerate() {
            if self.ended {
                break;
            }
            self.commands += 1;
            if self.commands > 100_000 {
                return None;
            }
            if context.literal(lexical.span.start) {
                continue;
            }
            if unsafe_control(&lexical.name) {
                return None;
            }
            if lexical.name == "endcsname" {
                continue;
            }
            let (name, span) = self
                .signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&lexical.name, lexical.span.clone()));
            if matches!(name, "csuse" | "csname") {
                return None;
            }
            if unsafe_control(name)
                || name.contains("refsection") && !STATE.contains(&name) && name != "therefsection"
            {
                return None;
            }
            if matches!(
                name,
                "setcounter"
                    | "addtocounter"
                    | "stepcounter"
                    | "refstepcounter"
                    | "defcounter"
                    | "counterwithin"
                    | "counterwithout"
                    | "@addtoreset"
                    | "@removefromreset"
            ) {
                let (counter, _) =
                    syntax::group(source, syntax::skip_space(source, span.end), b'{', b'}')?;
                if !syntax::is_static_key(&counter.value) || counter.value == "refsection" {
                    return None;
                }
            }
            let environment = if matches!(name, "begin" | "end") {
                Some(syntax::environment_name_argument(source, span.end)?)
            } else {
                None
            };
            let relevant = (STATE.contains(&name)
                && environment
                    .as_ref()
                    .is_none_or(|env| matches!(env.value.as_str(), "document" | "refsection")))
                || loader(name);
            if relevant {
                self.work += source.len() + doc.syntax.commands.len();
                if self.work > 32 * 1024 * 1024 {
                    return None;
                }
            }
            if relevant
                && (!self.signatures.command_unchanged(name)
                    || !positions.contains(&lexical.span.start)
                    || context.operand(doc, lexical.span.start)
                    || self.signatures.resource_is_tokenized(
                        source,
                        &doc.syntax,
                        lexical.span.start,
                    ))
            {
                // Deferred code and ordinary macro parameters can change the
                // state later; textual ordering cannot prove when they run.
                return None;
            }
            if uri == self.origin && lexical.span.start == self.query.command {
                if !self.document || !positions.contains(&lexical.span.start) {
                    return None;
                }
                if self
                    .answer
                    .as_ref()
                    .is_some_and(|previous| *previous != self.active)
                {
                    return None;
                }
                self.answer = Some(self.active.clone());
            }
            if !relevant {
                continue;
            }
            let command = Command {
                name: name.into(),
                span: span.clone(),
                argument: if span == lexical.span {
                    lexical.argument.clone()
                } else {
                    syntax::source_input_argument(source, name, span.end)
                },
            };
            if crate::bib_resources::references_file(name) {
                if self.document
                    || crate::resource_context::grouped(source, &doc.syntax, span.start)
                {
                    return None;
                }
            } else if matches!(name, "begin" | "end") {
                let environment = syntax::environment_name_argument(source, span.end)?;
                if environment.value == "document" {
                    if name == "begin" {
                        self.document = true;
                    } else {
                        self.ended = true;
                    }
                } else if environment.value == "refsection" {
                    if name == "begin" {
                        self.select(doc, environment.outer_span.end)?;
                    } else {
                        self.active = self.registry.resources(self.root, None)?;
                    }
                }
            } else if matches!(name, "newrefsection" | "refsection") {
                self.select(doc, span.end)?;
            } else if name == "endrefsection" {
                self.active = self.registry.resources(self.root, None)?;
            } else if loader(name) {
                let command = doc.syntax.potential_file_command(source, &command);
                let static_name = if syntax::file_list(name) {
                    syntax::complete_file_list_spans(source, &command).is_some()
                } else {
                    syntax::file_argument(source, &command)
                        .is_some_and(|argument| syntax::is_literal_file_path(&argument.value))
                };
                if !static_name {
                    return None;
                }
                let dependencies = self
                    .ws
                    .literal_command_dependencies(uri, self.root, &command);
                if matches!(name, "input" | "include" | "subfile") && dependencies.is_empty() {
                    return None;
                }
                for (dependency, kind) in dependencies {
                    if kind == DocumentKind::Latex {
                        self.walk(&dependency)?;
                    }
                }
            }
        }
        self.stack.remove(uri);
        Some(())
    }
    fn select(&mut self, doc: &Document, end: usize) -> Option<()> {
        if !self.document {
            return None;
        }
        let source = &doc.text.source;
        let at = syntax::skip_space(source, end);
        if crate::resource_context::ambiguous_header(&doc.syntax, at) {
            return None;
        }
        let selected = if source.as_bytes().get(at) == Some(&b'[') {
            let raw = doc.syntax.raw_resource_brackets.contains(&at);
            let (argument, _) = if raw {
                syntax::verbatim_file_bracket(source, at)
            } else {
                syntax::bracket_argument(source, at)
            }?;
            let segments = if raw {
                syntax::verbatim_list_segments(source, argument.span)
            } else {
                syntax::key_list_segments(source, argument.span)
            };
            let mut items = Vec::new();
            for span in segments {
                let value = source[span].trim();
                if value.is_empty() {
                    continue;
                }
                if !crate::bib_section_files::editable_item(value) {
                    return None;
                }
                items.push(value.to_owned());
            }
            Some(items)
        } else {
            None
        };
        self.active = self.registry.resources(self.root, selected.as_deref())?;
        Some(())
    }
}

struct Target {
    span: Range<usize>,
    resources: BTreeSet<Url>,
}
// If a resource analysis limit is reached, do not fall back to a union
// target that the ordinary key index would otherwise expose.
fn incomplete(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Target>> {
    crate::named_references::Index::new(ws, uri)
        .target(uri, offset)
        .filter(|(citation, _)| *citation)
        .map(|_| None)
}
/// None leaves ordinary projects to the existing symbol model; Some(None)
/// owns a section-dependent citation whose provenance cannot be certified.
fn target(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Target>> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)?;
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
        || !ws.index_truncated.is_empty()
    {
        return None;
    }
    let mut roots: BTreeSet<_> = ws.source_roots(uri).into_iter().collect();
    // Opened roots consume shared files even before their own path is saved.
    for candidate in ws
        .documents
        .keys()
        .filter(|candidate| ws.is_root(candidate))
    {
        if ws.project_documents(candidate).contains(uri) {
            roots.insert(candidate.clone());
        }
        if roots.len() > 64 {
            return incomplete(ws, uri, offset);
        }
    }
    let mut projects = Vec::new();
    let mut selected = false;
    let mut bytes = 0usize;
    for root in roots {
        let mut scope = ws.project_documents(&root);
        scope.insert(root.clone());
        let Some(docs) = crate::source_inputs::bounded_latex_sources(ws, &scope) else {
            return incomplete(ws, uri, offset);
        };
        bytes += docs
            .iter()
            .map(|(_, doc)| doc.text.source.len())
            .sum::<usize>();
        if bytes > 16 * 1024 * 1024 {
            return incomplete(ws, uri, offset);
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        selected |= crate::bib_sections::selects_resources(
            &docs.iter().map(|(_, doc)| *doc).collect::<Vec<_>>(),
            &signatures,
        );
        projects.push((root, docs, signatures));
    }
    if !selected
        || !projects
            .iter()
            .any(|(_, _, signatures)| citation_candidate(doc, signatures, offset))
    {
        return None;
    }
    Some((|| {
        let mut result: Option<Target> = None;
        for (root, docs, signatures) in &projects {
            let query = query(doc, signatures, offset)?;
            for (_, doc) in docs {
                if doc
                    .syntax
                    .contextual_resource_readers
                    .values()
                    .flatten()
                    .any(|reader| reader.opaque)
                {
                    return None;
                }
                let compact: String = doc
                    .text
                    .source
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect();
                if compact.contains("refsection=") || compact.contains("^^") {
                    return None;
                }
                if STATE.iter().any(|name| !signatures.command_unchanged(name)) {
                    return None;
                }
            }
            let registry = Registry::new(root, docs, signatures);
            let active = registry.resources(root, None)?;
            let mut analysis = Analysis {
                ws,
                root,
                signatures,
                registry,
                origin: uri,
                query: query.clone(),
                active,
                answer: None,
                visited: BTreeSet::new(),
                stack: BTreeSet::new(),
                visits: 0,
                commands: 0,
                work: 0,
                document: false,
                ended: false,
            };
            analysis.walk(root)?;
            // A potential input that was never reached must not contribute a
            // resource registration from the project's discovery graph.
            if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri)) {
                return None;
            }
            let resources = analysis.answer?;
            if resources.iter().any(|uri| {
                !ws.documents
                    .get(uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Bibtex)
            }) {
                return None;
            }
            if result.as_ref().is_some_and(|previous| {
                previous.span != query.span || previous.resources != resources
            }) {
                return None;
            }
            result = Some(Target {
                span: query.span,
                resources,
            });
        }
        result
    })())
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let Some(target) = target(ws, uri, offset)? else {
        return Some(Vec::new());
    };
    let name = &ws.documents[uri].text.source[target.span];
    Some(
        target
            .resources
            .iter()
            .flat_map(|uri| {
                let doc = &ws.documents[uri];
                doc.syntax
                    .citations
                    .iter()
                    .chain(&doc.syntax.bibliography_aliases)
                    .filter(move |entry| entry.name == name)
                    .map(move |entry| Location {
                        uri: uri.clone(),
                        range: doc.text.range(entry.span.start, entry.span.end),
                    })
            })
            .collect(),
    )
}
pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Hover>> {
    let target = target(ws, uri, offset)?;
    Some(target.and_then(|target| {
        let source = &ws.documents[uri];
        let name = &source.text.source[target.span.clone()];
        let mut entries = target.resources.iter().flat_map(|uri| {
            let doc = &ws.documents[uri];
            doc.syntax
                .citations
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
                .filter(move |entry| entry.name == name)
                .map(move |entry| (uri, doc, entry))
        });
        let (entry_uri, doc, entry) = entries.next()?;
        if entries.next().is_some() {
            return None;
        }
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: format!(
                    "{}\n\n{}\n{}:{}",
                    entry.name,
                    entry.detail,
                    entry_uri.path(),
                    doc.text.position(entry.span.start).line + 1
                ),
            }),
            range: Some(source.text.range(target.span.start, target.span.end)),
        })
    }))
}
