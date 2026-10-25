//! Generated commands and dependencies of environments, counters and conditionals.
use crate::{
    parser::syntax::{self, Argument, Command},
    project::workspace::{Document, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::Url,
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Environment,
    Counter,
    Conditional,
    Toggle,
}

#[derive(Clone, Copy)]
pub(crate) struct Generated {
    kind: Kind,
    numbered: bool,
    owns_counter: bool,
    alias: bool,
}

impl Generated {
    fn counter(alias: bool) -> Self {
        Self {
            kind: Kind::Counter,
            numbered: true,
            owns_counter: true,
            alias,
        }
    }
}

struct Declaration<'a> {
    name: &'a str,
    environment: Generated,
    shared: Option<Argument>,
    within: Option<Argument>,
}

fn declaration<'a>(doc: &'a Document, command: &Command) -> Option<Declaration<'a>> {
    if doc.syntax.control_operands.contains(&command.span.start) {
        return None;
    }
    if syntax::declares_toggle(&command.name) {
        let name = doc.text.source[command.argument.as_ref()?.span.clone()].trim();
        if !syntax::is_static_key(name) {
            return None;
        }
        return Some(Declaration {
            name,
            environment: Generated {
                kind: Kind::Toggle,
                numbered: false,
                owns_counter: false,
                alias: false,
            },
            shared: None,
            within: None,
        });
    }
    if let Some(span) =
        syntax::conditional_declaration(&doc.text.source, &doc.syntax.commands, command)
    {
        return Some(Declaration {
            name: &doc.text.source[span],
            environment: Generated {
                kind: Kind::Conditional,
                numbered: false,
                owns_counter: false,
                alias: false,
            },
            shared: None,
            within: None,
        });
    }
    if !syntax::declares_environment(&command.name)
        && !matches!(command.name.as_str(), "newcounter" | "newaliascnt")
    {
        return None;
    }
    let argument = command.argument.as_ref()?;
    let name = &doc.text.source[argument.span.clone()];
    if !syntax::is_static_key(name) {
        return None;
    }
    let source = &doc.text.source;
    if matches!(command.name.as_str(), "newcounter" | "newaliascnt") {
        return Some(Declaration {
            name,
            environment: Generated::counter(command.name == "newaliascnt"),
            shared: None,
            within: None,
        });
    }
    if command.name != "newtheorem" {
        return Some(Declaration {
            name,
            environment: Generated {
                kind: Kind::Environment,
                numbered: false,
                owns_counter: false,
                alias: false,
            },
            shared: None,
            within: None,
        });
    }
    let numbered = !source[command.span.end..argument.outer_span.start]
        .trim_start()
        .starts_with('*');
    let after = syntax::skip_space(source, argument.outer_span.end);
    let shared = crate::symbols::counters::optional_argument(source, after, 1);
    let title_start = shared
        .as_ref()
        .map_or(after, |(_, end)| syntax::skip_space(source, *end));
    let title = syntax::group(source, title_start, b'{', b'}');
    let within = if shared.is_none() {
        title
            .and_then(|(_, end)| {
                crate::symbols::counters::optional_argument(
                    source,
                    syntax::skip_space(source, end),
                    2,
                )
            })
            .map(|(argument, _)| argument)
    } else {
        None
    };
    Some(Declaration {
        name,
        environment: Generated {
            kind: Kind::Environment,
            numbered,
            owns_counter: numbered && shared.is_none(),
            alias: false,
        },
        shared: shared.map(|(argument, _)| argument),
        within,
    })
}

pub(crate) fn counters(doc: &Document) -> impl Iterator<Item = &str> {
    doc.syntax
        .commands
        .iter()
        .filter_map(|command| declaration(doc, command))
        .filter(|declaration| {
            declaration.environment.kind == Kind::Environment
                && declaration.environment.owns_counter
        })
        .map(|declaration| declaration.name)
}

pub(crate) fn in_scope(
    workspace: &Workspace,
    scope: &BTreeSet<Url>,
    name: &str,
    kind: Kind,
) -> Option<Generated> {
    workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .find_map(|(_, doc)| {
            doc.syntax
                .commands
                .iter()
                .filter_map(|command| declaration(doc, command))
                .find(|declaration| {
                    declaration.name == name && declaration.environment.kind == kind
                })
                .map(|declaration| declaration.environment)
        })
}

fn generated_names(name: &str, environment: Generated) -> Vec<(String, usize)> {
    let mut names = Vec::new();
    if environment.kind == Kind::Toggle {
        return vec![(format!("etb@tgl@{name}"), 8)];
    }
    if environment.kind == Kind::Conditional {
        return conditional_names(name).into_iter().zip([2, 0, 0]).collect();
    }
    if environment.kind == Kind::Environment {
        names.extend([(name.into(), 0), (format!("end{name}"), 3)]);
    }
    if environment.numbered {
        names.push((format!("the{name}"), 3));
    }
    if environment.owns_counter {
        for prefix in ["theH", "c@", "cl@", "p@"] {
            names.push((format!("{prefix}{name}"), prefix.len()));
        }
        names.push((format!("{name}autorefname"), 0));
    }
    if environment.alias {
        names.push((format!("AC@cnt@{name}"), 7));
    }
    names
}

pub(crate) fn conditional_names(name: &str) -> [String; 3] {
    [
        format!("if{name}"),
        format!("{name}true"),
        format!("{name}false"),
    ]
}

pub(crate) struct Reference {
    pub span: Range<usize>,
    pub selection_span: Range<usize>,
    control_word: bool,
}

pub(crate) fn references(doc: &Document, name: &str, environment: Generated) -> Vec<Reference> {
    let mut result = Vec::new();
    let generated = generated_names(name, environment);
    if environment.owns_counter {
        result.extend(
            doc.syntax
                .name_counter_references
                .iter()
                .filter(|entry| entry.name == name)
                .map(|entry| Reference {
                    span: entry.span.clone(),
                    selection_span: entry.span.clone(),
                    control_word: false,
                }),
        );
    }
    for entry in &doc.syntax.name_command_references {
        if let Some((_, prefix)) = generated
            .iter()
            .find(|(generated, _)| *generated == entry.name)
        {
            let control_word = doc.text.source[entry.span.clone()].starts_with('\\');
            let start = entry.span.start + usize::from(control_word) + prefix;
            result.push(Reference {
                span: start..start + name.len(),
                selection_span: entry.span.clone(),
                control_word,
            });
        }
    }
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if let Some((_, prefix)) = generated
            .iter()
            .find(|(generated, _)| generated == &command.name)
        {
            let start = command.span.start + 1 + prefix;
            result.push(Reference {
                span: start..start + name.len(),
                selection_span: command.span.clone(),
                control_word: true,
            });
        }
        if let Some(span) = syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index) {
            if let Some((_, prefix)) = generated
                .iter()
                .find(|(name, _)| *name == doc.text.source[span.clone()])
            {
                let start = span.start + prefix;
                result.push(Reference {
                    span: start..start + name.len(),
                    selection_span: span,
                    control_word: false,
                });
            }
        }
        for span in crate::symbols::named_commands::spans(doc, command) {
            if let Some((_, prefix)) = generated
                .iter()
                .find(|(name, _)| *name == doc.text.source[span.clone()])
            {
                let start = span.start + prefix;
                result.push(Reference {
                    span: start..start + name.len(),
                    selection_span: span,
                    control_word: false,
                });
            }
        }
        if doc.syntax.control_operands.contains(&command.span.start) {
            continue;
        }
        let mut arguments = Vec::new();
        if environment.kind == Kind::Environment {
            arguments.extend(syntax::environment_copy_source(&doc.text.source, command));
        }
        if matches!(environment.kind, Kind::Conditional | Kind::Toggle) {
            arguments.extend(crate::symbols::conditionals::expression_arguments(
                &doc.text.source,
                command,
                environment.kind == Kind::Toggle,
            ));
        }
        if environment.kind == Kind::Toggle
            && (syntax::declares_toggle(&command.name) || syntax::toggle_reference(&command.name))
        {
            arguments.extend(command.argument.iter().cloned());
        }
        if environment.kind == Kind::Conditional
            && (syntax::declares_boolean(&command.name) || syntax::boolean_reference(&command.name))
        {
            arguments.extend(command.argument.iter().cloned());
        }
        if matches!(environment.kind, Kind::Environment | Kind::Counter)
            && matches!(command.name.as_str(), "crefname" | "Crefname")
        {
            arguments.extend(command.argument.iter().cloned());
        }
        if environment.owns_counter {
            let counter = matches!(
                command.name.as_str(),
                "setcounter"
                    | "addtocounter"
                    | "defcounter"
                    | "ifltxcounter"
                    | "stepcounter"
                    | "refstepcounter"
                    | "usecounter"
                    | "value"
                    | "arabic"
                    | "roman"
                    | "Roman"
                    | "alph"
                    | "Alph"
                    | "fnsymbol"
                    | "aliascntresetthe"
            );
            let pair = matches!(
                command.name.as_str(),
                "counterwithin"
                    | "counterwithout"
                    | "numberwithin"
                    | "@addtoreset"
                    | "@removefromreset"
            ) && (doc.text.source.as_bytes().get(command.span.end) != Some(&b'*')
                || matches!(command.name.as_str(), "counterwithin" | "counterwithout"));
            let referenced_counter =
                syntax::counter_reference_label_start(&doc.text.source, command).is_some();
            let counter =
                counter && doc.text.source.as_bytes().get(command.span.end) != Some(&b'*');
            if counter || pair || referenced_counter {
                arguments.extend(command.argument.iter().cloned());
            }
            if pair {
                if let Some(first) = &command.argument {
                    arguments.extend(
                        syntax::group(
                            &doc.text.source,
                            syntax::skip_space(&doc.text.source, first.outer_span.end),
                            b'{',
                            b'}',
                        )
                        .map(|(argument, _)| argument),
                    );
                }
            }
            if let Some(declaration) = declaration(doc, command) {
                arguments.extend(declaration.shared);
                arguments.extend(declaration.within);
            }
            if command.name == "newcounter" {
                if let Some(first) = &command.argument {
                    arguments.extend(
                        crate::symbols::counters::optional_argument(
                            &doc.text.source,
                            syntax::skip_space(&doc.text.source, first.outer_span.end),
                            1,
                        )
                        .map(|(argument, _)| argument),
                    );
                }
            }
            if command.name == "newaliascnt" {
                if let Some(first) = &command.argument {
                    arguments.extend(
                        syntax::group(
                            &doc.text.source,
                            syntax::skip_space(&doc.text.source, first.outer_span.end),
                            b'{',
                            b'}',
                        )
                        .map(|(argument, _)| argument),
                    );
                }
            }
        }
        result.extend(
            arguments
                .into_iter()
                .filter(|argument| {
                    if matches!(environment.kind, Kind::Environment | Kind::Counter) {
                        argument.value == name
                    } else {
                        argument.value.trim() == name
                    }
                })
                .map(|argument| {
                    let span = if matches!(environment.kind, Kind::Environment | Kind::Counter) {
                        argument.span
                    } else {
                        syntax::trimmed_span(&doc.text.source, argument.span)
                    };
                    Reference {
                        span: span.clone(),
                        selection_span: span,
                        control_word: false,
                    }
                }),
        );
    }
    result
}

pub(crate) fn target<'a>(
    workspace: &'a Workspace,
    scope: &BTreeSet<Url>,
    doc: &Document,
    offset: usize,
    kind: Kind,
) -> Option<(&'a str, Range<usize>)> {
    let mut target = None;
    for (uri, definition) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        for declaration in definition
            .syntax
            .commands
            .iter()
            .filter_map(|command| declaration(definition, command))
            .filter(|declaration| declaration.environment.kind == kind)
        {
            if let Some(reference) = references(doc, declaration.name, declaration.environment)
                .into_iter()
                .find(|reference| {
                    reference.selection_span.start <= offset
                        && offset <= reference.selection_span.end
                })
            {
                if target
                    .as_ref()
                    .is_some_and(|(name, _)| *name != declaration.name)
                {
                    return None;
                }
                target = Some((declaration.name, reference.span));
            }
        }
    }
    target
}

pub(crate) fn validate(
    workspace: &Workspace,
    scope: &BTreeSet<Url>,
    old: &str,
    new: &str,
    environment: Generated,
) -> Result<()> {
    if !syntax::is_static_key(old) {
        return Err(Error::invalid_params("Renames require a static name"));
    }
    if environment.numbered && crate::symbols::counters::BUILTINS.contains(&new) {
        return Err(Error::invalid_params(format!(
            "The counter `{new}` is already defined"
        )));
    }
    if environment.kind == Kind::Counter
        && in_scope(workspace, scope, old, Kind::Environment)
            .is_some_and(|environment| environment.numbered)
    {
        return Err(Error::invalid_params(
            "Rename is ambiguous: a theorem and an independent counter share generated commands",
        ));
    }
    let generated = generated_names(new, environment);
    for (uri, doc) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        let counter_conflict = environment.numbered
            && doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "newcounter" | "newaliascnt")
                    && command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.value == new)
            });
        let macro_conflict = generated.iter().any(|(name, _)| {
            doc.syntax
                .definitions
                .iter()
                .any(|entry| entry.name == *name)
                || generated_command_exists(doc, name)
                || crate::project::catalog::COMMANDS
                    .iter()
                    .any(|(command, _, _)| command == name)
                || crate::project::catalog::PRIMITIVES.contains(&name.as_str())
                || syntax::declares_register(name)
        });
        if counter_conflict || macro_conflict {
            return Err(Error::invalid_params(format!(
                "The name `{new}` conflicts with a counter or generated command"
            )));
        }
        if !new
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
            && references(doc, old, environment)
                .iter()
                .any(|reference| reference.control_word)
        {
            return Err(Error::invalid_params(
                "Names used in generated commands must contain only TeX command letters",
            ));
        }
    }
    Ok(())
}

pub(crate) fn generated_command_exists(doc: &Document, name: &str) -> bool {
    doc.syntax
        .commands
        .iter()
        .filter_map(|command| declaration(doc, command))
        .any(|declaration| {
            generated_names(declaration.name, declaration.environment)
                .iter()
                .any(|(generated, _)| generated == name)
        })
        || crate::symbols::counters::BUILTINS.iter().any(|counter| {
            generated_names(counter, Generated::counter(false))
                .iter()
                .any(|(generated, _)| generated == name)
        })
}

pub(crate) fn commands(doc: &Document) -> impl Iterator<Item = String> + '_ {
    doc.syntax
        .commands
        .iter()
        .filter_map(|command| declaration(doc, command))
        .flat_map(|declaration| generated_names(declaration.name, declaration.environment))
        .map(|(name, _)| name)
}
