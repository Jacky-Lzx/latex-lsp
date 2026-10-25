//! Bounded file spelling corrections for confirmed static source and code inputs.
use crate::{
    compilation::build_log,
    editing::spelling_fixes,
    parser::syntax,
    project::workspace::{self, Workspace},
};
use tower_lsp::lsp_types::{Diagnostic, NumberOrString, TextEdit, Url};

pub(crate) fn fixes(
    workspace: &Workspace,
    root: &Url,
    uri: &Url,
    diagnostic: &Diagnostic,
    signatures: Option<&crate::language::signature::CommandSignatures<'_>>,
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
    let confirmed = if matches!(code, "missing-input" | "missing-package" | "missing-class") {
        let Some(signatures) = signatures else {
            return vec![];
        };
        Some(build_log::file_error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures,
        ))
    } else {
        None
    };
    let command_name = match code {
        "missing-input" => "input",
        "missing-listing" => "lstinputlisting",
        "missing-verbatim-input" => "verbatiminput",
        "missing-fancyverb-input" => "VerbatimInput",
        "missing-minted-input" => "inputminted",
        "missing-include" => "include",
        "missing-bibliography-style" => "bibliographystyle",
        "missing-bibliography-database" => {
            let Some(command) = diagnostic
                .data
                .as_ref()
                .and_then(|data| data.get("bibliographyCommand"))
                .and_then(|value| value.as_str())
                .filter(|command| {
                    matches!(*command, "bibliography" | "nobibliography")
                        || crate::bibliography::bib_resources::loads(command)
                })
            else {
                return vec![];
            };
            command
        }
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
    let listing = command_name == "lstinputlisting";
    let fancyverb = crate::parser::fancyvrb::is_input(command_name);
    let code_input = syntax::code_input_command(command_name);
    let database = matches!(command_name, "bibliography" | "nobibliography");
    let resource = crate::bibliography::bib_resources::loads(command_name);
    let extension = loader_extension.unwrap_or(match command_name {
        "bibliographystyle" => "bst",
        "bibliography" | "nobibliography" => "bib",
        name if crate::bibliography::bib_resources::loads(name) => "",
        name if syntax::code_input_command(name) => "",
        _ => "tex",
    });
    let compact = syntax::file_name_ignores_spaces(command_name);
    let reported = if fancyverb {
        build_log::missing_fancyverb_name(&diagnostic.message)
    } else if command_name == "inputminted" {
        build_log::missing_minted_name(&diagnostic.message)
            .or_else(|| build_log::missing_fancyverb_name(&diagnostic.message))
    } else if command_name == "verbatiminput" {
        build_log::missing_verbatim_name(&diagnostic.message)
    } else if listing {
        build_log::missing_listing_name(&diagnostic.message)
    } else if command_name == "bibliographystyle" {
        build_log::missing_bibliography_style_name(&diagnostic.message)
    } else if database || resource {
        build_log::missing_bibliography_database_name(&diagnostic.message)
    } else if command_name != "include" {
        build_log::missing_file_name(&diagnostic.message)
    } else {
        build_log::missing_include_key(&diagnostic.message)
    };
    let reported = reported.filter(|name| {
        if command_name == "include" {
            return true;
        }

        let file = diagnostic
            .data
            .as_ref()
            .and_then(|data| data.get("file"))
            .and_then(|name| name.as_str());
        if listing {
            file.is_some_and(|file| crate::parser::listings::missing_report(file) == *name)
        } else {
            file == Some(*name)
        }
    });
    let Some(reported) = reported else {
        return vec![];
    };
    let indirect = if let Some(signatures) = signatures.filter(|_| {
        matches!(
            command_name,
            "input" | "include" | "bibliography" | "nobibliography" | "bibliographystyle"
        ) || resource
            || syntax::loader_extension(command_name).is_some()
    }) {
        let commands = if database || resource || command_name == "bibliographystyle" {
            let positions = signatures.source_command_positions(&doc.text.source, &doc.syntax);
            signatures
                .indirect_bibliography_invocations(&doc.text.source, &doc.syntax, &positions)
                .into_iter()
                .filter(|call| call.certain)
                .map(|call| call.command)
                .collect()
        } else {
            build_log::file_error_commands(&doc.text.source, &doc.syntax, signatures)
        };
        let index = (!commands.is_empty())
            .then(|| crate::project::source_inputs::Index::new(workspace, root));
        commands
            .into_iter()
            .filter(|command| {
                index.as_ref().is_some_and(|index| {
                    index
                        .calls
                        .get(uri)
                        .into_iter()
                        .flatten()
                        .chain(index.loaders.get(uri).into_iter().flatten())
                        .chain(index.resources.get(uri).into_iter().flatten())
                        .any(|call| {
                            call.certain
                                && call.command.name == command.name
                                && call.command.span == command.span
                                && call
                                    .command
                                    .argument
                                    .as_ref()
                                    .zip(command.argument.as_ref())
                                    .is_some_and(|(a, b)| {
                                        a.span == b.span
                                            && a.outer_span == b.outer_span
                                            && a.value == b.value
                                    })
                        })
                })
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    let active_resource = if resource {
        let Some(signatures) =
            signatures.filter(|signatures| signatures.command_unchanged(command_name))
        else {
            return vec![];
        };
        Some(signatures.source_command_positions(&doc.text.source, &doc.syntax))
    } else {
        None
    };
    let argument = doc
        .syntax
        .commands
        .iter()
        .chain(&indirect)
        .filter(|command| {
            (command.name == command_name
                || fancyverb && crate::parser::fancyvrb::is_input(&command.name))
                && active_resource.as_ref().is_none_or(|positions| {
                    positions.contains(&command.span.start)
                        || indirect
                            .iter()
                            .any(|call| call.span == command.span && call.name == command.name)
                })
        })
        .find_map(|command| {
            if code_input && !crate::project::code_inputs::supported(workspace, uri, root, command)
            {
                return None;
            }
            let argument = syntax::file_argument(&doc.text.source, command)?;
            let spans = if syntax::file_list(command_name) {
                syntax::complete_file_list_spans(&doc.text.source, command)?
            } else {
                vec![syntax::trimmed_span(
                    &doc.text.source,
                    argument.span.clone(),
                )]
            };
            if loader_extension.is_some()
                && spans.iter().any(|span| {
                    let raw = &doc.text.source[span.clone()];
                    let literal = if compact {
                        syntax::loader_name(raw)
                    } else {
                        raw.into()
                    };
                    !syntax::is_literal_file_path(&literal)
                })
            {
                return None;
            }
            for span in spans {
                let value = &doc.text.source[span.clone()];
                let matches_report = if command_name == "verbatiminput"
                    || fancyverb
                    || resource
                    || command_name == "inputminted"
                {
                    value == reported
                } else if listing {
                    crate::parser::listings::missing_report(value) == reported
                } else if command_name == "bibliographystyle" {
                    format!("{value}.bst") == reported
                } else {
                    syntax::file_lookup_names(command_name, value).any(|name| name == reported)
                };
                if doc.text.range(span.start, span.end) == diagnostic.range
                    && matches_report
                    && confirmed
                        .as_ref()
                        .is_none_or(|calls| calls.iter().any(|(candidate, _)| candidate == &span))
                {
                    return Some((argument.into_owned(), span, command));
                }
            }
            None
        });
    let Some((argument, span, command)) = argument else {
        return vec![];
    };
    let command_name = command.name.as_str();
    let value = &doc.text.source[span];
    let literal = compact.then(|| syntax::loader_name(value));
    if !(if resource {
        crate::bibliography::bib_resources::literal_path(value)
    } else {
        syntax::is_literal_file_path(literal.as_deref().unwrap_or(value))
    }) || workspace::resolve_file_path(root, command_name, value, &[extension]).is_some()
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
    let normalized = compact.then(|| syntax::loader_name(value));
    let lookup = normalized.as_deref().unwrap_or(value);
    let separator = lookup.rfind('/').map_or(0, |index| index + 1);
    let prefix = &lookup[..separator];
    let original = if code_input || resource || loader_extension.is_some() {
        &lookup[separator..]
    } else {
        lookup[separator..]
            .strip_suffix(&format!(".{extension}"))
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
    let filters = if command_name == "include" {
        let Some(filters) = crate::project::source_inputs::selection_filters(
            workspace,
            &workspace.project_documents(root),
        ) else {
            return vec![];
        };
        filters
    } else {
        vec![]
    };
    let mut candidates = Vec::new();
    let allow_spaces = command_name != "inputminted"
        || crate::project::code_inputs::allows_spaces(workspace, uri, root);
    for entry in files.take(4096).flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(stem) = (if resource {
            let expected = std::path::Path::new(original)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("bib");
            (std::path::Path::new(&name)
                .extension()
                .and_then(|value| value.to_str())
                == Some(expected))
            .then_some(name.as_str())
        } else if code_input {
            Some(name.as_str())
        } else {
            name.strip_suffix(&format!(".{extension}"))
        }) else {
            continue;
        };
        if !allow_spaces && name.chars().any(char::is_whitespace)
            || !(if resource {
                crate::bibliography::bib_resources::literal_path(&name)
            } else {
                syntax::is_literal_file_path(&name)
            })
            || name.starts_with('.') && !original.starts_with('.')
            || (command_name == "bibliographystyle" || database)
                && name.chars().any(char::is_whitespace)
            || database && name.contains(',')
        {
            continue;
        }
        if *comparisons >= spelling_fixes::MAX_COMPARISONS {
            break;
        }
        *comparisons += 1;
        // A frequent loader mistake is spelling the complete file name; the
        // loader then appends its suffix a second time.
        let score = if loader_extension.is_some() && name == original
            || resource && name == format!("{original}.bib")
        {
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
            if code_input
                || resource
                || loader_extension.is_none() && lookup.ends_with(&format!(".{extension}"))
            {
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
        if !code_input && (target == source || target == main) {
            continue;
        }
        candidates.push((score, replacement, target));
    }
    candidates.sort();
    let mut fixes = Vec::new();
    // Check the nearest candidates only; retain a bound on dependency traversals.
    for (_, replacement, target) in candidates.into_iter().take(32) {
        if !code_input {
            let dependencies = workspace.project_documents_from(&target, root);
            if dependencies.contains(uri)
                || dependencies.contains(root)
                || dependencies.contains(&source)
                || dependencies.contains(&main)
            {
                continue;
            }
        }
        let edit = if crate::project::code_inputs::wrapped_filename(command_name) {
            let Some(edit) =
                crate::project::code_inputs::filename_edit(&doc.text, command, replacement.clone())
            else {
                continue;
            };
            edit
        } else if command_name == "input"
            && syntax::input_filename_needs_braces(&doc.text.source, &argument, &replacement)
        {
            syntax::input_filename_edit(&doc.text, &argument, replacement.clone())
        } else {
            TextEdit {
                range: diagnostic.range,
                new_text: replacement.clone(),
            }
        };
        fixes.push(syntax::QuickFix {
            title: format!(
                "Replace with {} file `{replacement}`",
                if command_name == "input" {
                    "input"
                } else if code_input {
                    "code"
                } else if loader_extension == Some("sty") {
                    "package"
                } else if loader_extension == Some("cls") {
                    "class"
                } else if command_name == "bibliographystyle" {
                    "bibliography style"
                } else if database || resource {
                    "bibliography database"
                } else {
                    "included"
                }
            ),
            diagnostic: diagnostic.clone(),
            edit,
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
    use crate::project::workspace::Document;

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
        let _ = fixes(&workspace, &uri, &uri, &diagnostic, None, &mut comparisons);
        assert_eq!(comparisons, spelling_fixes::MAX_COMPARISONS);
        assert!(fixes(&workspace, &uri, &uri, &diagnostic, None, &mut comparisons).is_empty());
        assert_eq!(comparisons, spelling_fixes::MAX_COMPARISONS);
    }
}
