//! Bounded file spelling corrections for compiler-confirmed static source inputs.
use crate::{
    build_log, spelling_fixes, syntax,
    workspace::{self, Workspace},
};
use tower_lsp::lsp_types::{Diagnostic, NumberOrString, TextEdit, Url};

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
    let code = match diagnostic.code.as_ref() {
        Some(NumberOrString::String(code)) => code.as_str(),
        _ => return vec![],
    };
    let command_name = match code {
        "missing-input" => "input",
        "missing-include" => "include",
        "missing-package" | "missing-class" => {
            let Some(command) = diagnostic
                .data
                .as_ref()
                .and_then(|data| data.get("loaderCommand"))
                .and_then(|value| value.as_str())
            else {
                return vec![];
            };
            let expected = if code == "missing-package" {
                "sty"
            } else {
                "cls"
            };
            if syntax::loader_extension(command) != Some(expected) {
                return vec![];
            }
            command
        }
        _ => return vec![],
    };
    let loader_extension = syntax::loader_extension(command_name);
    let extension = loader_extension.unwrap_or("tex");
    let reported = if command_name != "include" {
        build_log::missing_file_name(&diagnostic.message).filter(|name| {
            diagnostic
                .data
                .as_ref()
                .and_then(|data| data.get("file"))
                .and_then(|name| name.as_str())
                == Some(*name)
        })
    } else {
        build_log::missing_include_key(&diagnostic.message)
    };
    let Some(reported) = reported else {
        return vec![];
    };
    let argument = doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == command_name)
        .filter_map(|command| command.argument.as_ref())
        .find_map(|argument| {
            let values = if syntax::file_list(command_name) {
                argument.value.split(',').collect::<Vec<_>>()
            } else {
                vec![argument.value.as_str()]
            };
            if loader_extension.is_some()
                && values
                    .iter()
                    .any(|value| !syntax::is_literal_file_path(&syntax::loader_name(value.trim())))
            {
                return None;
            }
            let mut offset = argument.span.start;
            for value in values {
                let span = syntax::trimmed_span(&doc.text.source, offset..offset + value.len());
                if doc.text.range(span.start, span.end) == diagnostic.range
                    && syntax::file_lookup_names(command_name, value.trim())
                        .any(|name| name == reported)
                {
                    return Some((argument, span));
                }
                offset += value.len() + 1;
            }
            None
        });
    let Some((argument, span)) = argument else {
        return vec![];
    };
    let value = &doc.text.source[span];
    let literal = loader_extension.map(|_| syntax::loader_name(value));
    if !syntax::is_literal_file_path(literal.as_deref().unwrap_or(value))
        || workspace::resolve_file_path(root, command_name, value, &[extension]).is_some()
        || command_name == "input"
            && doc.text.source[argument.outer_span.clone()].starts_with('"')
            && !doc.text.source[argument.outer_span.clone()].ends_with('"')
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
    let normalized = loader_extension.map(|_| syntax::loader_name(value));
    let lookup = normalized.as_deref().unwrap_or(value);
    let separator = lookup.rfind('/').map_or(0, |index| index + 1);
    let prefix = &lookup[..separator];
    let original = if loader_extension.is_some() {
        &lookup[separator..]
    } else {
        lookup[separator..]
            .strip_suffix(".tex")
            .unwrap_or(&lookup[separator..])
    };
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
    if command_name == "include" {
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
                    if !syntax::is_literal_file_path(value) {
                        return vec![];
                    }
                    names.push(value.strip_suffix(".tex").unwrap_or(value).to_owned());
                }
                filters.push(names);
            }
        }
    }
    let mut candidates = Vec::new();
    for entry in files.take(4096).flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(stem) = name.strip_suffix(&format!(".{extension}")) else {
            continue;
        };
        if !syntax::is_literal_file_path(&name)
            || name.starts_with('.') && !original.starts_with('.')
        {
            continue;
        }
        if *comparisons >= spelling_fixes::MAX_COMPARISONS {
            break;
        }
        *comparisons += 1;
        // A frequent loader mistake is spelling the complete file name; the
        // loader then appends its suffix a second time.
        let score = if loader_extension.is_some() && name == original {
            Some(1)
        } else {
            spelling_fixes::distance(original, stem, if length < 8 { 1 } else { 2 })
        };
        let Some(score) = score else {
            continue;
        };
        if score == 0 || !entry.path().is_file() {
            continue;
        }
        let replacement = format!(
            "{prefix}{}",
            if loader_extension.is_none() && value.ends_with(".tex") {
                &name
            } else {
                stem
            }
        );
        let included_name = replacement.strip_suffix(".tex").unwrap_or(&replacement);
        if filters
            .iter()
            .any(|names| !names.iter().any(|name| name == included_name))
        {
            continue;
        }
        let Some(target) =
            workspace::resolve_file_path(root, command_name, &replacement, &[extension])
        else {
            continue;
        };
        let candidate = entry
            .path()
            .canonicalize()
            .ok()
            .and_then(|path| Url::from_file_path(path).ok());
        if candidate.as_ref() != Some(&target) {
            continue;
        }
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
        let new_text = if command_name == "input"
            && argument.span == argument.outer_span
            && replacement.contains(' ')
        {
            format!("{{{replacement}}}")
        } else {
            replacement.clone()
        };
        fixes.push(syntax::QuickFix {
            title: format!(
                "Replace with {} file `{replacement}`",
                if command_name == "input" {
                    "input"
                } else if loader_extension == Some("sty") {
                    "package"
                } else if loader_extension == Some("cls") {
                    "class"
                } else {
                    "included"
                }
            ),
            diagnostic: diagnostic.clone(),
            edit: TextEdit {
                range: diagnostic.range,
                new_text,
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
            code: Some(NumberOrString::String("missing-include".into())),
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
