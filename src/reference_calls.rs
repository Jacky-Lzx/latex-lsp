//! Static named reference calls shared by diagnostics and editing.
use crate::{signature::CommandSignatures, syntax};
use std::{collections::BTreeSet, ops::Range};

pub(crate) fn named_command(
    source: &str,
    parsed: &syntax::Syntax,
    index: usize,
    signatures: &CommandSignatures<'_>,
    operands: &BTreeSet<usize>,
    declarations: &BTreeSet<(usize, usize)>,
) -> Option<syntax::Command> {
    let lexical = parsed.commands.get(index)?;
    let (name, span) = if let Some(invocation) =
        signatures.named_invocation(source, parsed, index, operands, declarations)
    {
        invocation
    } else {
        // Builtin-name redefinitions keep their conservative ordinary-macro
        // treatment. This path recognizes distinct user forwarding names.
        if syntax::is_reference(&lexical.name)
            || syntax::has_separate_label_argument(&lexical.name)
            || syntax::is_citation(&lexical.name)
        {
            return None;
        }
        (
            signatures.forwarded_builtin(&lexical.name)?,
            lexical.span.clone(),
        )
    };
    let name = if syntax::is_reference(name)
        || syntax::has_separate_label_argument(name)
        || syntax::is_citation(name)
    {
        name
    } else {
        signatures.forwarded_builtin(name)?
    };
    if !syntax::is_reference(name)
        && !syntax::has_separate_label_argument(name)
        && !syntax::is_citation(name)
    {
        return None;
    }
    let start = if syntax::is_reference(name) {
        syntax::reference_argument_start(source, name, span.end)
    } else if syntax::is_citation(name) {
        syntax::citation_argument_start(source, name, span.end)
    } else {
        None
    };
    let argument = start
        .and_then(|at| syntax::group(source, at, b'{', b'}'))
        .map(|(argument, _)| {
            if syntax::is_single_reference(name) {
                syntax::single_reference_argument(source, argument)
            } else if syntax::is_single_citation(name) {
                syntax::single_citation_argument(source, argument)
            } else {
                argument
            }
        });
    Some(syntax::Command {
        name: name.into(),
        span,
        argument,
    })
}

pub(crate) fn keys(
    source: &str,
    command: &syntax::Command,
    warnings: bool,
) -> Vec<(bool, Range<usize>)> {
    let mut result = Vec::new();
    if syntax::is_citation(&command.name)
        && (!warnings || syntax::warns_on_undefined_citation(&command.name))
    {
        if let Some(style) = crate::citations::style(&command.name) {
            for arg in crate::citations::arguments(source, command.span.end, style).keys() {
                result.extend(
                    crate::citations::key_spans(source, arg, style)
                        .into_iter()
                        .map(|span| (true, span)),
                );
            }
        } else if let Some(arg) = &command.argument {
            if syntax::is_single_citation(&command.name) {
                if syntax::is_static_citation_key(&arg.value) {
                    result.push((true, arg.span.clone()));
                }
            } else {
                result.extend(
                    syntax::citation_list_spans(source, arg.span.clone())
                        .into_iter()
                        .map(|span| (true, span)),
                );
            }
        }
    } else if (!warnings && syntax::is_reference(&command.name))
        || syntax::warns_on_undefined_reference(&command.name)
        || syntax::has_separate_label_argument(&command.name)
    {
        if let Some(arg) = command
            .argument
            .as_ref()
            .filter(|_| !syntax::has_separate_label_argument(&command.name))
        {
            if syntax::is_single_reference(&command.name) {
                if syntax::is_static_reference_key(&arg.value) {
                    result.push((false, arg.span.clone()));
                }
            } else {
                result.extend(
                    syntax::reference_list_spans(source, arg.span.clone())
                        .into_iter()
                        .map(|span| (false, span)),
                );
            }
        }
        if let Some(arg) = syntax::separate_label_argument(source, command)
            .or_else(|| syntax::range_reference_end(source, command))
        {
            if syntax::is_static_reference_key(&arg.value) {
                result.push((false, arg.span));
            }
        }
    }
    result
}
