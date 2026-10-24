//! Hint-only certification of literal entry-context assignments.
use crate::syntax;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub sorting: Option<String>,
    pub prefix: String,
    pub hashed_prefix: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Assignment {
    pub before: usize,
    pub section: u32,
    pub selection: Selection,
    pub keys: Vec<String>,
    pub weak: bool,
}

pub(crate) struct Parsed {
    pub selection: Selection,
    pub keys: Vec<String>,
    pub weak: bool,
}

// Resolve the actual invocation before inspecting its star: a weak assignment
// can occur after a context has already opened, including through an alias.
pub(crate) fn has_weak(
    doc: &crate::workspace::Document,
    signatures: &crate::signature::CommandSignatures<'_>,
) -> bool {
    let source = &doc.text.source;
    let context = crate::named_references::Context::new(doc, signatures);
    doc.syntax
        .commands
        .iter()
        .enumerate()
        .any(|(index, command)| {
            let (name, span) = signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            signatures.forwarded_builtin(name).unwrap_or(name) == "assignrefcontextentries"
                && source.as_bytes().get(syntax::skip_space(source, span.end)) == Some(&b'*')
        })
}

pub(crate) fn parse(source: &str, end: usize) -> Option<Parsed> {
    let mut at = syntax::skip_space(source, end);
    let weak = source.as_bytes().get(at) == Some(&b'*');
    if weak {
        at = syntax::skip_space(source, at + 1);
    }
    if source.as_bytes().get(at) != Some(&b'[') {
        return None;
    }
    let end = syntax::optional_note_end(source, at)?;
    let options = &source[at + 1..end - 1];
    if options.contains(['%', '\\', '#'])
        || options
            .chars()
            .any(|ch| ch.is_whitespace() && !ch.is_ascii())
    {
        return None;
    }
    let mut selection = Selection {
        sorting: None,
        prefix: String::new(),
        hashed_prefix: false,
    };
    let mut seen = BTreeSet::new();
    for span in syntax::key_list_segments(options, 0..options.len()) {
        let option = options[span].trim();
        if option.is_empty() {
            continue;
        }
        let (name, raw) = option.split_once('=')?;
        let name = name.trim();
        if !seen.insert(name) {
            return None;
        }
        let raw = raw.trim();
        let value = if raw.starts_with('{') {
            let (argument, end) = syntax::group(raw, 0, b'{', b'}')?;
            if end != raw.len() {
                return None;
            }
            argument.value
        } else {
            raw.to_string()
        };
        match name {
            "sorting" if syntax::is_static_key(&value) && !value.contains('/') => {
                selection.sorting = Some(value)
            }
            "labelprefix" if crate::bib_context::flat_prefix(&value) => {
                selection.prefix = value;
                selection.hashed_prefix = true;
            }
            _ => return None,
        }
    }
    let (argument, _) = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?;
    if argument
        .value
        .chars()
        .any(|ch| ch.is_whitespace() && !ch.is_ascii())
    {
        return None;
    }
    let mut keys = Vec::new();
    for key in argument
        .value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        if !syntax::is_static_citation_key(key) || keys.len() >= 100_000 {
            return None;
        }
        keys.push(key.to_string());
    }
    Some(Parsed {
        selection,
        keys,
        weak,
    })
}

type Numbers = BTreeMap<(u32, String, String), Option<String>>;
type Labels = BTreeMap<(u32, String, bool, String, String), Option<String>>;

pub(crate) struct Compiled {
    labels: Labels,
    pub work: usize,
}
impl Compiled {
    // Index every loaded entry once. Different compiled contexts with the
    // same literal selection remain ambiguous even if their labels agree.
    pub(crate) fn new(numbers: &Numbers, prefixes: crate::bib_bbl::Prefixes) -> Option<Self> {
        let mut labels = Labels::new();
        let mut work = 0usize;
        for ((section, context, key), prefix) in prefixes {
            let fields: Vec<_> = context.split('/').collect();
            if fields.len() != 6 || ![1, 3, 4, 5].iter().all(|index| fields[*index] == "global") {
                continue;
            }
            let number = numbers
                .get(&(section, key.clone(), context.clone()))
                .cloned()
                .flatten();
            let label = number.and_then(|number| {
                let label = format!("{prefix}{number}");
                (label.chars().count() <= 256).then_some(label)
            });
            work += key.len()
                + prefix.len()
                + fields[0].len()
                + label.as_ref().map_or(0, String::len)
                + 192;
            if work > 32 * 1024 * 1024 {
                return None;
            }
            match labels.entry((
                section,
                fields[0].to_string(),
                !fields[2].is_empty(),
                prefix,
                key,
            )) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(label);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
            }
        }
        Some(Self { labels, work })
    }
    pub(crate) fn label(
        &self,
        section: u32,
        key: &str,
        selection: &Selection,
        global_sorting: Option<&str>,
    ) -> Option<String> {
        let sorting = selection.sorting.as_deref().or(global_sorting)?;
        self.labels
            .get(&(
                section,
                sorting.to_string(),
                selection.hashed_prefix,
                selection.prefix.clone(),
                key.to_string(),
            ))
            .cloned()
            .flatten()
    }
}

// The initial assignment proof requires a literal package sorting option
// when the assignment does not specify its own. Other context defaults are
// native global templates; named/template options are rejected by parse().
pub(crate) fn global_sorting(
    docs: &[(tower_lsp::lsp_types::Url, &crate::workspace::Document)],
    signatures: &crate::signature::CommandSignatures<'_>,
) -> Option<String> {
    let mut sorting = None;
    for (_, doc) in docs {
        let source = &doc.text.source;
        let context = crate::named_references::Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        for command in &doc.syntax.commands {
            if !matches!(command.name.as_str(), "usepackage" | "RequirePackage")
                || context.literal(command.span.start)
                || context.operand(doc, command.span.start)
                || !positions.contains(&command.span.start)
                || !signatures.command_unchanged(&command.name)
                || !syntax::file_argument(source, command)
                    .is_some_and(|argument| argument.value.trim() == "biblatex")
            {
                continue;
            }
            let at = syntax::skip_space(source, command.span.end);
            if source.as_bytes().get(at) != Some(&b'[') {
                return None;
            }
            let end = syntax::optional_note_end(source, at)?;
            let options = &source[at + 1..end - 1];
            for span in syntax::key_list_segments(options, 0..options.len()) {
                let Some((name, raw)) = options[span].split_once('=') else {
                    continue;
                };
                if name.trim() != "sorting" {
                    continue;
                }
                let raw = raw.trim();
                let value = if raw.starts_with('{') {
                    let (argument, end) = syntax::group(raw, 0, b'{', b'}')?;
                    if end != raw.len() {
                        return None;
                    }
                    argument.value
                } else {
                    raw.to_string()
                };
                if !syntax::is_static_key(&value) || value.contains('/') {
                    return None;
                }
                sorting = Some(value);
            }
        }
    }
    sorting
}
