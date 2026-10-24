//! Nondeferred labels follow the actually loaded, certified BBL entry list.
use crate::{syntax, workspace::Workspace};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};
use tower_lsp::lsp_types::Url;

type Labels = BTreeMap<(u32, String), Option<String>>;

fn fields(source: &str, at: &mut usize, count: usize) -> Option<Vec<syntax::Argument>> {
    let mut result = Vec::new();
    for _ in 0..count {
        let (argument, end) = syntax::group(source, syntax::skip_space(source, *at), b'{', b'}')?;
        *at = end;
        result.push(argument);
    }
    Some(result)
}
fn command<'a>(source: &'a str, at: &mut usize) -> Option<&'a str> {
    *at = syntax::skip_space(source, *at);
    let word = syntax::control_word(source, *at)?;
    *at = word.end;
    Some(&source[word])
}
fn decimal(value: &str) -> Option<u32> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}
fn context(value: &str) -> bool {
    let components: Vec<_> = value.split('/').collect();
    value.len() <= 256
        && components.len() == 6
        && components[2].is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-:".contains(&byte))
}
fn compact(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}
fn parse(source: &str) -> Option<Labels> {
    let mut lines = source.lines();
    if lines.next()? != "% $ biblatex auxiliary file $"
        || lines.next()? != "% $ biblatex bbl format version 3.3 $"
    {
        return None;
    }
    // Match the generated preface, including its nonexecuted error branch.
    // An arbitrary command before the first section cannot certify numbering.
    let mut at = 0;
    if command(source, &mut at)? != r"\begingroup"
        || command(source, &mut at)? != r"\makeatletter"
        || command(source, &mut at)? != r"\@ifundefined"
    {
        return None;
    }
    let preface = fields(source, &mut at, 3)?;
    if preface[0].value != "ver@biblatex.sty"
        || compact(&preface[1].value)
            != compact(
                r"\@latex@error{Missing 'biblatex' package}{The bibliography requires the 'biblatex' package.}\aftergroup\endinput",
            )
        || !preface[2].value.trim().is_empty()
        || command(source, &mut at)? != r"\endgroup"
    {
        return None;
    }
    let mut section = None;
    let mut sections = BTreeSet::new();
    let mut listed = false;
    let mut in_list = false;
    let mut entry = None;
    let mut counter = 0u32;
    let mut labels = BTreeMap::new();
    loop {
        match command(source, &mut at)? {
            r"\refsection" => {
                if section.is_some() || sections.len() >= 4096 {
                    return None;
                }
                let args = fields(source, &mut at, 1)?;
                let value = decimal(&args[0].value)?;
                if !sections.insert(value) {
                    return None;
                }
                section = Some(value);
                listed = false;
            }
            r"\datalist" => {
                if section.is_none() || listed || in_list {
                    return None;
                }
                let (kind, end) = syntax::bracket_argument(source, syntax::skip_space(source, at))?;
                at = end;
                let args = fields(source, &mut at, 1)?;
                if kind.value != "entry" || !context(&args[0].value) {
                    return None;
                }
                listed = true;
                in_list = true;
                counter = 0;
            }
            r"\entry" => {
                if !in_list || entry.is_some() || labels.len() >= 4096 {
                    return None;
                }
                let args = fields(source, &mut at, 4)?;
                let key = &args[0].value;
                if !syntax::is_static_citation_key(key)
                    || key == "*"
                    || !syntax::is_static_key(&args[1].value)
                    || args[1].value.eq_ignore_ascii_case("set")
                    || !args[2].value.trim().is_empty()
                    || !args[3].value.is_empty() && decimal(&args[3].value).is_none()
                {
                    return None;
                }
                entry = Some(key.clone());
                counter = counter.checked_add(1)?;
            }
            r"\field" => {
                entry.as_ref()?;
                let args = fields(source, &mut at, 2)?;
                if matches!(
                    args[0].value.to_ascii_lowercase().as_str(),
                    "shorthand"
                        | "labelnumber"
                        | "localnumber"
                        | "labelprefix"
                        | "options"
                        | "entrykey"
                        | "entrytype"
                        | "entryset"
                ) {
                    return None;
                }
            }
            name @ (r"\name" | r"\list" | r"\strng" | r"\range" | r"\keyw" | r"\true"
            | r"\false" | r"\warn") => {
                entry.as_ref()?;
                let count = match name {
                    r"\name" => 4,
                    r"\list" => 3,
                    r"\strng" | r"\range" => 2,
                    _ => 1,
                };
                fields(source, &mut at, count)?;
            }
            r"\endentry" => {
                let key = entry.take()?;
                if labels
                    .insert((section?, key), Some(counter.to_string()))
                    .is_some()
                {
                    return None;
                }
            }
            r"\enddatalist" => {
                if !in_list || entry.is_some() {
                    return None;
                }
                in_list = false;
            }
            r"\endrefsection" => {
                if !listed || in_list || entry.is_some() {
                    return None;
                }
                section.take()?;
            }
            r"\endinput" => {
                if section.is_some()
                    || in_list
                    || entry.is_some()
                    || sections.is_empty()
                    || syntax::skip_space(source, at) != source.len()
                {
                    return None;
                }
                return Some(labels);
            }
            // Sets, aliases, verbatim data and every unmodeled record remain
            // unproved; never return labels from a partially recognized file.
            _ => return None,
        }
    }
}

pub(crate) fn numeric_labels(ws: &Workspace, root: &Url) -> Option<(Labels, usize)> {
    let root_path = root.to_file_path().ok()?;
    let path = ws
        .build_config(root)
        .artifact_path(&root_path, "bbl")
        .filter(|path| path.is_file())
        .unwrap_or_else(|| root_path.with_extension("bbl"));
    if std::fs::metadata(&path).ok()?.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut source = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut source)
        .ok()?;
    if source.len() > 2 * 1024 * 1024 {
        return None;
    }
    Some((parse(&source)?, source.len()))
}

#[cfg(test)]
#[path = "bib_bbl_tests.rs"]
mod tests;
