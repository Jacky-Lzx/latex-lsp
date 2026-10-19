//! File spelling corrections only for compiler-confirmed static include warnings.
use crate::{
    build_log, spelling_fixes, syntax,
    workspace::{self, Workspace},
};
use tower_lsp::lsp_types::{Diagnostic, TextEdit, Url};

fn literal_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && !value.starts_with(' ')
        && !value.ends_with(' ')
        && !value.contains("  ")
        && !value.chars().any(|ch| ch.is_whitespace() && ch != ' ')
        && !value.contains([
            '\\', '{', '}', '%', '#', '$', '&', '^', '~', '"', '\r', '\n', '\0',
        ])
}

pub(crate) fn fixes(
    workspace: &Workspace,
    root: &Url,
    uri: &Url,
    diagnostic: &Diagnostic,
    comparisons: &mut usize,
) -> Vec<syntax::QuickFix> {
    if *comparisons >= spelling_fixes::MAX_COMPARISONS {
        return vec![];
    }
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(reported) = build_log::missing_include_key(&diagnostic.message) else {
        return vec![];
    };
    let argument = doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "include")
        .filter_map(|command| command.argument.as_ref())
        .find(|argument| {
            let span = syntax::trimmed_span(&doc.text.source, argument.span.clone());
            doc.text.range(span.start, span.end) == diagnostic.range
                && syntax::file_lookup_names("include", argument.value.trim())
                    .any(|name| name == reported)
        });
    let Some(argument) = argument else {
        return vec![];
    };
    let value = argument.value.trim();
    if !literal_path(value)
        || workspace::resolve_file_path(root, "include", value, &["tex"]).is_some()
    {
        return vec![];
    }
    let Some(parent) = root
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_owned()))
    else {
        return vec![];
    };
    let separator = value.rfind('/').map_or(0, |index| index + 1);
    let prefix = &value[..separator];
    let original = value[separator..]
        .strip_suffix(".tex")
        .unwrap_or(&value[separator..]);
    let length = original.chars().count();
    if !(3..=128).contains(&length) {
        return vec![];
    }
    let Ok(files) = std::fs::read_dir(parent.join(prefix)) else {
        return vec![];
    };
    let canonical_uri = |uri: &Url| {
        uri.to_file_path()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .and_then(|path| Url::from_file_path(path).ok())
            .unwrap_or_else(|| uri.clone())
    };
    let source = canonical_uri(uri);
    let main = canonical_uri(root);
    let mut filters = Vec::new();
    for document in workspace
        .project_documents(root)
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
    {
        for command in document
            .syntax
            .commands
            .iter()
            .filter(|command| command.name == "includeonly")
        {
            let Some(argument) = &command.argument else {
                return vec![];
            };
            let mut names = Vec::new();
            for value in argument
                .value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !literal_path(value) {
                    return vec![];
                }
                names.push(value.strip_suffix(".tex").unwrap_or(value).to_owned());
            }
            filters.push(names);
        }
    }
    let mut candidates = Vec::new();
    for entry in files.take(4096).flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".tex") else {
            continue;
        };
        if !literal_path(&name) || name.starts_with('.') && !original.starts_with('.') {
            continue;
        }
        if *comparisons >= spelling_fixes::MAX_COMPARISONS {
            break;
        }
        *comparisons += 1;
        let Some(score) = spelling_fixes::distance(original, stem, if length < 8 { 1 } else { 2 })
        else {
            continue;
        };
        if score == 0 || !entry.path().is_file() {
            continue;
        }
        let replacement = format!(
            "{prefix}{}",
            if value.ends_with(".tex") { &name } else { stem }
        );
        let included_name = replacement.strip_suffix(".tex").unwrap_or(&replacement);
        if filters
            .iter()
            .any(|names| !names.iter().any(|name| name == included_name))
        {
            continue;
        }
        let Some(target) = workspace::resolve_file_path(root, "include", &replacement, &["tex"])
        else {
            continue;
        };
        if target == source || target == main {
            continue;
        }
        candidates.push((score, replacement, target));
    }
    candidates.sort();
    let mut fixes = Vec::new();
    // Check the nearest candidates only; retain a bound on dependency traversals.
    for (_, replacement, target) in candidates.into_iter().take(32) {
        let dependencies = workspace.project_documents_from(&target, root);
        if dependencies.contains(uri)
            || dependencies.contains(root)
            || dependencies.contains(&source)
            || dependencies.contains(&main)
        {
            continue;
        }
        fixes.push(syntax::QuickFix {
            title: format!("Replace with included file `{replacement}`"),
            diagnostic: diagnostic.clone(),
            edit: TextEdit {
                range: diagnostic.range,
                new_text: replacement,
            },
            additional_edits: vec![],
        });
        if fixes.len() == 5 {
            break;
        }
    }
    fixes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Document;

    #[test]
    fn file_spelling_comparisons_share_and_respect_the_remaining_request_budget() {
        let directory = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let source = "\\include{chapetr}";
        std::fs::write(directory.path().join("main.tex"), source).unwrap();
        std::fs::write(directory.path().join("chapter.tex"), "Chapter.").unwrap();
        let mut workspace = Workspace::default();
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        let diagnostic = Diagnostic {
            range: workspace.documents[&uri].text.range(9, 16),
            message: "Included file `chapetr.tex` not found".into(),
            ..Default::default()
        };
        let mut comparisons = spelling_fixes::MAX_COMPARISONS - 1;
        let _ = fixes(&workspace, &uri, &uri, &diagnostic, &mut comparisons);
        assert_eq!(comparisons, spelling_fixes::MAX_COMPARISONS);
        assert!(fixes(&workspace, &uri, &uri, &diagnostic, &mut comparisons).is_empty());
        assert_eq!(comparisons, spelling_fixes::MAX_COMPARISONS);
    }
}
