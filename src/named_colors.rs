//! Literal xcolor names form a namespace separate from macros and reference keys.
use crate::{
    syntax::{self, Argument, Command, Entry, Syntax},
    workspace::Workspace,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};
use tower_lsp::{jsonrpc, lsp_types::*};

pub(crate) const BUILTINS: &[&str] = &[
    "black",
    "blue",
    "brown",
    "cyan",
    "darkgray",
    "gray",
    "green",
    "lightgray",
    "lime",
    "magenta",
    "olive",
    "orange",
    "pink",
    "purple",
    "red",
    "teal",
    "violet",
    "white",
    "yellow",
];

pub(crate) fn builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

pub(crate) fn literal_name(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Definition,
    Expression,
    Blend,
    Mask,
}

struct Field {
    start: usize,
    role: Role,
}

struct Reader<'a> {
    source: &'a str,
    at: usize,
    fields: Vec<Field>,
}

impl Reader<'_> {
    fn group(&mut self) -> Result<Argument, ()> {
        let (argument, end) = syntax::group(
            self.source,
            syntax::skip_space(self.source, self.at),
            b'{',
            b'}',
        )
        .ok_or(())?;
        self.at = end;
        Ok(argument)
    }
    fn optional(&mut self) -> Result<Option<Argument>, ()> {
        self.at = syntax::skip_space(self.source, self.at);
        if self.source.as_bytes().get(self.at) != Some(&b'[') {
            return Ok(None);
        }
        let (argument, end) = syntax::bracket_argument(self.source, self.at).ok_or(())?;
        self.at = end;
        Ok(Some(argument))
    }
    fn field(&mut self, role: Role) -> Result<Argument, ()> {
        let start = syntax::skip_space(self.source, self.at);
        if self.source.as_bytes().get(start) != Some(&b'{') {
            return Err(());
        }
        self.fields.push(Field { start, role });
        self.group()
    }
    fn class(&mut self) -> Result<(), ()> {
        if self
            .optional()?
            .is_some_and(|a| !matches!(a.value.as_str(), "" | "named" | "ps"))
        {
            return Err(());
        }
        Ok(())
    }
    fn model(&mut self) -> Result<Option<String>, ()> {
        let model = self.optional()?.map(|a| a.value);
        if model.as_deref().is_some_and(|m| !known_model(m)) {
            return Err(());
        }
        Ok(model)
    }
    fn color(&mut self, model: Option<&str>) -> Result<(), ()> {
        if model == Some("") {
            return Err(());
        }
        if model.is_none_or(|m| m == "named") {
            self.field(Role::Expression)?;
        } else {
            self.group()?;
        }
        Ok(())
    }
}

fn known_model(model: &str) -> bool {
    matches!(
        model,
        "" | "named"
            | "rgb"
            | "RGB"
            | "HTML"
            | "cmy"
            | "cmyk"
            | "gray"
            | "Gray"
            | "hsb"
            | "Hsb"
            | "tHsb"
            | "HSB"
            | "wave"
    )
}

fn known(name: &str) -> bool {
    matches!(
        name,
        "definecolor"
            | "providecolor"
            | "xdefinecolor"
            | "colorlet"
            | "color"
            | "textcolor"
            | "colorbox"
            | "fcolorbox"
            | "pagecolor"
            | "rowcolor"
            | "columncolor"
            | "cellcolor"
            | "arrayrulecolor"
            | "doublerulesepcolor"
            | "blendcolors"
            | "maskcolors"
            | "extractcolorspec"
            | "extractcolorspecs"
            | "convertcolorspec"
    )
}

fn read(reader: &mut Reader<'_>, name: &str) -> Result<(), ()> {
    match name {
        "definecolor" | "providecolor" | "xdefinecolor" => {
            reader.class()?;
            reader.field(Role::Definition)?;
            if reader
                .optional()?
                .is_some_and(|a| !a.value.is_empty() && !syntax::is_static_key(&a.value))
            {
                return Err(());
            }
            let model = reader.group()?;
            if !known_model(&model.value) || model.value.is_empty() {
                return Err(());
            }
            if model.value == "named" {
                reader.field(Role::Expression)?;
            } else {
                reader.group()?;
            }
        }
        "colorlet" => {
            reader.class()?;
            reader.field(Role::Definition)?;
            reader.model()?;
            reader.field(Role::Expression)?;
        }
        "fcolorbox" => {
            let frame = reader.model()?;
            reader.color(frame.as_deref())?;
            let background = reader.model()?;
            reader.color(background.as_deref().or(frame.as_deref()))?;
        }
        "extractcolorspec" | "extractcolorspecs" => {
            reader.field(Role::Expression)?;
        }
        "convertcolorspec" => {
            let model = reader.group()?;
            if !known_model(&model.value) || model.value.is_empty() {
                return Err(());
            }
            reader.color(Some(&model.value))?;
            if !known_model(&reader.group()?.value) {
                return Err(());
            }
        }
        "maskcolors" => {
            reader.model()?;
            reader.field(Role::Mask)?;
        }
        "blendcolors" => {
            reader.at = syntax::skip_space(reader.source, reader.at);
            if reader.source.as_bytes().get(reader.at) == Some(&b'*') {
                reader.at += 1;
            }
            reader.field(Role::Blend)?;
        }
        _ => {
            let model = reader.model()?;
            reader.color(model.as_deref())?;
        }
    }
    Ok(())
}

fn fields(source: &str, command: &Command) -> (Vec<Field>, bool) {
    let mut reader = Reader {
        source,
        at: command.span.end,
        fields: vec![],
    };
    let complete = read(&mut reader, &command.name).is_ok();
    (reader.fields, complete)
}

fn percentage(value: &str) -> bool {
    value.bytes().any(|b| b.is_ascii_digit())
        && value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && value
            .parse::<f64>()
            .is_ok_and(|n| n.is_finite() && (0.0..=100.0).contains(&n))
}

/// Keep complements and mixing ratios outside the editable name spans.
fn expression(source: &str, span: Span<usize>) -> Option<Vec<Span<usize>>> {
    let mut result = vec![];
    let mut start = span.start;
    for (index, part) in source[span.clone()].split('!').enumerate() {
        if index % 2 == 1 {
            if !percentage(part) {
                return None;
            }
        } else {
            let name = part.trim_start_matches('-');
            if name != "." {
                if !literal_name(name) {
                    return None;
                }
                let name_start = start + part.len() - name.len();
                result.push(name_start..start + part.len());
            }
        }
        start += part.len() + 1;
    }
    Some(result)
}

fn field_expression(source: &str, mut span: Span<usize>, role: Role) -> Option<Vec<Span<usize>>> {
    if matches!(role, Role::Blend | Role::Mask) && span.is_empty() {
        return Some(vec![]);
    }
    if role == Role::Blend {
        if !source[span.clone()].starts_with('!') {
            return None;
        }
        span.start += 1;
        let end = source[span.clone()]
            .find('!')
            .map_or(span.end, |n| span.start + n);
        if !percentage(&source[span.start..end]) {
            return None;
        }
        if end == span.end {
            return Some(vec![]);
        }
        span.start = end + 1;
    }
    expression(source, span)
}

fn occurrences(source: &str, command: &Command) -> Option<Vec<(bool, Span<usize>)>> {
    let (fields, complete) = fields(source, command);
    if !complete {
        return None;
    }
    let mut result = vec![];
    for field in fields {
        let (argument, _) = syntax::group(source, field.start, b'{', b'}')?;
        match field.role {
            Role::Definition => {
                if !literal_name(&argument.value) {
                    return None;
                }
                result.push((true, argument.span));
            }
            role => result.extend(
                field_expression(source, argument.span, role)?
                    .into_iter()
                    .map(|span| (false, span)),
            ),
        }
    }
    Some(result)
}

/// Direct calls whose complete consumed arguments can be checked against TeX's
/// source echo. Blend settings are evaluated later, so they have no direct site.
pub(crate) fn error_invocations(source: &str, syntax: &Syntax) -> Vec<(Span<usize>, Span<usize>)> {
    if syntax
        .definitions
        .iter()
        .any(|entry| known(&entry.name) || unsupported(&entry.name))
    {
        return vec![];
    }
    let mut result = vec![];
    for command in &syntax.commands {
        let (body, outputs) = match command.name.as_str() {
            "color" | "pagecolor" | "colorlet" | "definecolor" | "providecolor"
            | "xdefinecolor" | "maskcolors" => (false, 0),
            "textcolor" | "colorbox" | "fcolorbox" => (true, 0),
            "extractcolorspec" | "convertcolorspec" => (false, 1),
            "extractcolorspecs" => (false, 2),
            _ => continue,
        };
        let mut reader = Reader {
            source,
            at: command.span.end,
            fields: vec![],
        };
        if read(&mut reader, &command.name).is_err() {
            continue;
        }
        let complete = if body {
            reader.group().is_ok()
        } else {
            (0..outputs).all(|_| {
                let start = syntax::skip_space(source, reader.at);
                if let Some(word) = syntax::control_word(source, start) {
                    reader.at = word.end;
                    true
                } else {
                    false
                }
            })
        };
        if !complete {
            continue;
        }
        let tail = syntax::skip_space(source, command.span.end)..reader.at;
        if let Some(entries) = occurrences(source, command) {
            result.extend(
                entries
                    .into_iter()
                    .filter(|(definition, _)| !definition)
                    .map(|(_, span)| (span, tail.clone())),
            );
        }
    }
    result
}

pub(crate) fn index(source: &str, syntax: &mut Syntax) {
    for command in syntax.commands.iter().filter(|c| known(&c.name)) {
        if let Some(occurrences) = occurrences(source, command) {
            for (declaration, span) in occurrences {
                let entry = Entry {
                    name: source[span.clone()].into(),
                    span,
                    detail: format!(
                        "Color {} with \\{}",
                        if declaration { "defined" } else { "referenced" },
                        command.name
                    ),
                    is_variable: false,
                };
                if declaration {
                    syntax.colors.push(entry);
                } else {
                    syntax.color_references.push(entry);
                }
            }
        }
    }
}

fn unsupported(name: &str) -> bool {
    matches!(
        name,
        "DefineNamedColor"
            | "definecolors"
            | "providecolors"
            | "definecolorset"
            | "providecolorset"
            | "preparecolor"
            | "preparecolorset"
            | "definecolorseries"
            | "resetcolorseries"
            | "hypersetup"
            | "tikzset"
            | "pgfplotsset"
            | "tcbset"
            | "newtcolorbox"
            | "tikz"
            | "draw"
            | "fill"
            | "filldraw"
            | "path"
            | "node"
            | "shade"
            | "shadedraw"
            | "addplot"
            | "addplot3"
            | "colornameprefix"
            | "colorblend"
            | "colormask"
            | "XC@definecolor"
    ) || name.starts_with("XC@")
        || name.trim_start_matches('\\').starts_with("color@")
}

// Package options can contain color consumers that are outside the literal
// command grammar. Refuse a partial rename when such settings are present.
fn color_options(source: &str, command: &Command) -> bool {
    let mut reader = Reader {
        source,
        at: command.span.end,
        fields: vec![],
    };
    let options = match command.name.as_str() {
        "usepackage" | "RequirePackage" | "documentclass" | "LoadClass" => {
            match reader.optional() {
                Ok(options) => options,
                Err(()) => return true,
            }
        }
        "PassOptionsToPackage" | "PassOptionsToClass" => match reader.group() {
            Ok(options) => Some(options),
            Err(()) => return true,
        },
        _ => return false,
    };
    options.is_some_and(|a| {
        a.value.contains(['\\', '#'])
            || a.value.split(',').any(|part| {
                let key = part
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                key.contains("color") && part.contains('=')
            })
    })
}

pub(crate) fn validate(workspace: &Workspace, scope: &BTreeSet<Url>) -> jsonrpc::Result<()> {
    for (_, doc) in workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
    {
        if doc
            .syntax
            .definitions
            .iter()
            .any(|e| known(&e.name) || unsupported(&e.name))
        {
            return Err(jsonrpc::Error::invalid_params(
                "Color rename cannot resolve redefined color commands",
            ));
        }
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let unsafe_name = |name: &str| known(name) || unsupported(name);
            let indirect = crate::named_commands::spans(doc, command)
                .iter()
                .any(|s| unsafe_name(&doc.text.source[s.clone()]))
                || syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index)
                    .is_some_and(|s| unsafe_name(&doc.text.source[s]));
            let unsupported = unsupported(&command.name)
                || command.name == "begin"
                    && command.argument.as_ref().is_some_and(|a| {
                        matches!(
                            a.value.as_str(),
                            "tikzpicture" | "axis" | "tcolorbox" | "scope"
                        )
                    })
                || color_options(&doc.text.source, command);
            if indirect
                || unsupported
                || known(&command.name) && occurrences(&doc.text.source, command).is_none()
            {
                return Err(jsonrpc::Error::invalid_params("Color rename cannot resolve dynamic names, indirect calls or unsupported color settings"));
            }
        }
    }
    Ok(())
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    for command in doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|c| known(&c.name) && c.span.end <= offset)
    {
        for field in fields(source, command).0 {
            let mut start = field.start + 1;
            let end = syntax::group(source, field.start, b'{', b'}')
                .map(|(a, _)| a.span.end)
                .unwrap_or_else(|| {
                    source[start..]
                        .find(['\n', '\r', '}'])
                        .map_or(source.len(), |n| start + n)
                });
            if offset < start || offset > end {
                continue;
            }
            if field.role == Role::Blend {
                if source.as_bytes().get(start) != Some(&b'!') {
                    return Some(vec![]);
                }
                let Some(n) = source[start + 1..end].find('!') else {
                    return Some(vec![]);
                };
                let separator = start + 1 + n;
                if !percentage(&source[start + 1..separator]) || offset <= separator {
                    return Some(vec![]);
                }
                start = separator + 1;
            }
            if field.role == Role::Definition {
                return Some(vec![]);
            }
            let before = &source[start..offset];
            let separators = before.bytes().filter(|b| *b == b'!').count();
            if separators % 2 == 1 {
                return Some(vec![]);
            }
            let mut at = before.rfind('!').map_or(start, |n| start + n + 1);
            if at > start && expression(source, start..at - 1).is_none() {
                return Some(vec![]);
            }
            let segment_end = source[at..end].find('!').map_or(end, |n| at + n);
            at += source[at..segment_end]
                .bytes()
                .take_while(|b| *b == b'-')
                .count();
            if offset < at {
                return Some(vec![]);
            }
            let prefix = &source[at..offset];
            if !prefix.is_empty() && !literal_name(prefix) {
                return Some(vec![]);
            }
            if !source[offset..segment_end]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Some(vec![]);
            }
            let mut names: BTreeMap<String, String> = BUILTINS
                .iter()
                .map(|n| (n.to_string(), "Built-in xcolor name".into()))
                .collect();
            for (_, document) in workspace
                .documents
                .iter()
                .filter(|(uri, _)| scope.contains(*uri))
            {
                for entry in &document.syntax.colors {
                    names.insert(entry.name.clone(), entry.detail.clone());
                }
            }
            return Some(
                names
                    .into_iter()
                    .filter(|(name, _)| name.starts_with(prefix))
                    .map(|(name, detail)| CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::COLOR),
                        detail: Some(detail),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range: doc.text.range(at, segment_end),
                            new_text: name,
                        })),
                        ..Default::default()
                    })
                    .collect(),
            );
        }
    }
    None
}
