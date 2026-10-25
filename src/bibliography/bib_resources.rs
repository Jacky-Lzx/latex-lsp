//! Static local biblatex resources use full, verbatim filenames rather than lists.
use crate::parser::syntax::{self, Argument, Command};
use std::ops::Range;

pub(crate) fn changes_when_tokenized(value: &str) -> bool {
    value.contains(['%', '#']) || value.contains("^^")
}

/// File-verb catcodes cannot recover comments, parameter tokens or TeX's
/// character translation after an enclosing macro has collected the filename.
pub(crate) fn insertion_preserves_filename(
    workspace: &crate::project::workspace::Workspace,
    uri: &tower_lsp::lsp_types::Url,
    roots: &[tower_lsp::lsp_types::Url],
    at: Option<usize>,
    value: &str,
) -> bool {
    if !changes_when_tokenized(value) {
        return true;
    }
    let Some((doc, at)) = workspace.documents.get(uri).zip(at) else {
        return false;
    };
    !roots.is_empty()
        && roots.iter().all(|root| {
            let mut scope = workspace.project_documents(root);
            scope.insert(uri.clone());
            let Some(documents) =
                crate::project::source_inputs::bounded_latex_sources(workspace, &scope)
            else {
                return false;
            };
            let signatures = crate::language::signature::CommandSignatures::source_context(
                documents
                    .iter()
                    .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
            );
            !signatures.resource_is_tokenized(&doc.text.source, &doc.syntax, at)
        })
}

pub(crate) fn loads(name: &str) -> bool {
    matches!(name, "addbibresource" | "addglobalbib")
        || crate::bibliography::bib_section_files::resource_command(name)
}

/// Both loading and registration commands expose a literal file reference.
pub(crate) fn references_file(name: &str) -> bool {
    loads(name) || name == "addsectionbib"
}

pub(crate) fn exact_filename(name: &str) -> bool {
    references_file(name) && name != crate::bibliography::bib_section_files::INFERRED
}

pub(crate) fn start(source: &str, command_end: usize) -> Option<usize> {
    let mut at = syntax::skip_space(source, command_end);
    if source.as_bytes().get(at) == Some(&b'*') {
        return None;
    }
    if source.as_bytes().get(at) == Some(&b'[') {
        at = syntax::skip_space(source, syntax::optional_note_end(source, at)?);
    }
    Some(at)
}

pub(crate) fn read_argument(source: &str, end: usize, tokenized: bool) -> Option<Argument> {
    let at = start(source, end)?;
    let (argument, _) = if tokenized {
        syntax::group(source, at, b'{', b'}')
    } else {
        syntax::verbatim_file_group(source, at)
    }?;
    (!tokenized || !changes_when_tokenized(&argument.value)).then_some(argument)
}

fn local_options(source: &str, command_end: usize) -> bool {
    let at = syntax::skip_space(source, command_end);
    if source.as_bytes().get(at) != Some(&b'[') {
        return true;
    }
    let Some(end) = syntax::optional_note_end(source, at) else {
        return false;
    };
    let options = &source[at + 1..end - 1];
    if options.contains(['%', '#']) {
        return false;
    }
    syntax::key_list_segments(options, 0..options.len())
        .into_iter()
        .all(|span| {
            let option = options[span].trim();
            if option.is_empty() {
                return true;
            }
            let Some((key, value)) = option.split_once('=') else {
                return false;
            };
            let value = value.trim();
            let value = value
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .unwrap_or(value);
            match key.trim() {
                "type" => value == "file",
                "datatype" => value == "bibtex",
                "location" => value == "local",
                "glob" => value == "false",
                "bibencoding" => !value.is_empty() && !value.contains(['\\', '{', '}']),
                // biblatex detokenizes labels; display macros cannot change
                // which local file this resource selects.
                "label" => !value.is_empty(),
                _ => false,
            }
        })
}

pub(crate) fn literal_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && !value.starts_with(' ')
        && !value.ends_with(' ')
        && !value.contains("  ")
        && !value.chars().any(char::is_control)
        && !value.contains(['\\', '{', '}', '"'])
}

pub(crate) fn argument(source: &str, command: &Command) -> Option<Argument> {
    if crate::bibliography::bib_section_files::resource_command(&command.name) {
        return command
            .argument
            .clone()
            .filter(|argument| literal_path(&argument.value));
    }
    if !local_options(source, command.span.end) {
        return None;
    }
    let argument = command.argument.clone()?;
    if argument.outer_span.start != start(source, command.span.end)? {
        return None;
    }
    literal_path(&argument.value).then_some(argument)
}

pub(crate) fn prefix(source: &str, command: &Command, cursor: usize) -> Option<Range<usize>> {
    if crate::bibliography::bib_section_files::file_command(&command.name) {
        let argument = command.argument.as_ref()?;
        return (argument.span.start <= cursor && cursor <= argument.span.end)
            .then_some(argument.span.start..cursor);
    }
    if !local_options(source, command.span.end) {
        return None;
    }
    let at = start(source, command.span.end)?;
    let value = source.get(at..cursor)?.strip_prefix('{')?;
    // An unfinished resource is still completable, but never look through a
    // closed group, macro, or source whitespace that would change its identity.
    (value.is_empty() || literal_path(value)).then_some(at + 1..cursor)
}
