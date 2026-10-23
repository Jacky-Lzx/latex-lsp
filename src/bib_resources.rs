//! Static local biblatex resources use full, verbatim filenames rather than lists.
use crate::syntax::{self, Argument, Command};
use std::ops::Range;

pub(crate) fn loads(name: &str) -> bool {
    matches!(name, "addbibresource" | "addglobalbib")
}

/// Both loading and registration commands expose a literal file reference.
pub(crate) fn references_file(name: &str) -> bool {
    loads(name) || name == "addsectionbib"
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
    if !local_options(source, command.span.end) {
        return None;
    }
    let (argument, _) = syntax::verbatim_file_group(source, start(source, command.span.end)?)?;
    literal_path(&argument.value).then_some(argument)
}

pub(crate) fn prefix(source: &str, command: &Command, cursor: usize) -> Option<Range<usize>> {
    if !local_options(source, command.span.end) {
        return None;
    }
    let at = start(source, command.span.end)?;
    let value = source.get(at..cursor)?.strip_prefix('{')?;
    // An unfinished resource is still completable, but never look through a
    // closed group, macro, or source whitespace that would change its identity.
    (value.is_empty() || literal_path(value)).then_some(at + 1..cursor)
}
