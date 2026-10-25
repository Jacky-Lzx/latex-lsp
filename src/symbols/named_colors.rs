//! Literal xcolor names form a namespace separate from macros and reference keys.
use crate::rpc as jsonrpc;
use crate::{
    parser::syntax::{self, Argument, Command, Entry, Syntax},
    project::workspace::Workspace,
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};

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

/// TeX discards a comment's line ending and the following line's indentation.
/// Keep every other byte, including literal whitespace, and retain source
/// positions so edits never consume a comment embedded in a color operand.
struct Tokens {
    value: String,
    positions: Vec<usize>,
}

impl Tokens {
    fn new(source: &str, span: Span<usize>) -> Self {
        let bytes = source.as_bytes();
        let mut positions = vec![];
        let mut at = span.start;
        while at < span.end {
            if bytes[at] == b'%' {
                while at < span.end && !matches!(bytes[at], b'\r' | b'\n') {
                    at += 1;
                }
                if at < span.end && bytes[at] == b'\r' {
                    at += 1;
                }
                if at < span.end && bytes[at] == b'\n' {
                    at += 1;
                }
                while at < span.end && matches!(bytes[at], b' ' | b'\t') {
                    at += 1;
                }
            } else {
                positions.push(at);
                // Escaped percent signs are tokens, never comments. Keeping
                // the backslash also makes dynamic operands fail validation.
                if bytes[at] == b'\\' && at + 1 < span.end {
                    at += 1;
                    positions.push(at);
                }
                at += 1;
            }
        }
        let value = String::from_utf8(positions.iter().map(|p| bytes[*p]).collect())
            .expect("comment removal preserves UTF-8");
        Self { value, positions }
    }

    fn span(&self, span: Span<usize>) -> Option<Span<usize>> {
        let positions = self.positions.get(span)?;
        let first = *positions.first()?;
        positions
            .iter()
            .enumerate()
            .all(|(i, p)| *p == first + i)
            .then_some(first..first + positions.len())
    }

    fn cursor(&self, offset: usize) -> Option<usize> {
        let at = self.positions.partition_point(|p| *p < offset);
        (self.positions.get(at) == Some(&offset) || at > 0 && self.positions[at - 1] + 1 == offset)
            .then_some(at)
    }
}

fn tokens(source: &str, argument: &Argument) -> String {
    Tokens::new(source, argument.span.clone()).value
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
            .is_some_and(|a| !matches!(tokens(self.source, &a).as_str(), "" | "named" | "ps"))
        {
            return Err(());
        }
        Ok(())
    }
    fn model(&mut self) -> Result<Option<String>, ()> {
        let model = self.optional()?.map(|a| tokens(self.source, &a));
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

pub(crate) const MODELS: &[&str] = &[
    "named", "rgb", "RGB", "HTML", "cmy", "cmyk", "gray", "Gray", "hsb", "Hsb", "tHsb", "HSB",
    "wave",
];

fn known_model(model: &str) -> bool {
    model.is_empty() || MODELS.contains(&model)
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
            if reader.optional()?.is_some_and(|a| {
                let value = tokens(reader.source, &a);
                !value.is_empty() && !syntax::is_static_key(&value)
            }) {
                return Err(());
            }
            let argument = reader.group()?;
            let model = tokens(reader.source, &argument);
            if !known_model(&model) || model.is_empty() {
                return Err(());
            }
            if model == "named" {
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
            let argument = reader.group()?;
            let model = tokens(reader.source, &argument);
            if !known_model(&model) || model.is_empty() {
                return Err(());
            }
            reader.color(Some(&model))?;
            let target = reader.group()?;
            if !known_model(&tokens(reader.source, &target)) {
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
        let tokens = Tokens::new(source, argument.span);
        match field.role {
            Role::Definition => {
                if !literal_name(&tokens.value) {
                    return None;
                }
                result.push((true, tokens.span(0..tokens.value.len())?));
            }
            role => result.extend(
                field_expression(&tokens.value, 0..tokens.value.len(), role)?
                    .into_iter()
                    .map(|span| tokens.span(span).map(|span| (false, span)))
                    .collect::<Option<Vec<_>>>()?,
            ),
        }
    }
    Some(result)
}

/// Direct calls whose complete consumed arguments can be checked against TeX's
/// source echo. Blend settings are evaluated later, so they have no direct site.
pub(crate) fn error_invocations_with_signatures(
    source: &str,
    syntax: &Syntax,
    signatures: &crate::language::signature::CommandSignatures<'_>,
) -> Vec<(Span<usize>, Span<usize>)> {
    if signatures
        .names()
        .any(|name| known(name) || unsupported(name))
    {
        return vec![];
    }
    let operands = signatures.argument_tokens(source, syntax);
    let mut result = vec![];
    for command in &syntax.commands {
        if operands.contains(&command.span.start) {
            continue;
        }
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

/// Parse model operands even when their spelling is invalid. Keep this separate
/// from symbol indexing, which must reject unknown color models.
pub(crate) fn model_error_invocations_with_signatures(
    source: &str,
    syntax: &Syntax,
    signatures: &crate::language::signature::CommandSignatures<'_>,
) -> Vec<(Span<usize>, Span<usize>)> {
    if signatures
        .names()
        .any(|name| known(name) || unsupported(name))
    {
        return vec![];
    }
    fn value(reader: &mut Reader<'_>) -> Result<(), ()> {
        let argument = reader.group()?;
        if argument.value.contains(['\\', '#', '%']) {
            return Err(());
        }
        Ok(())
    }
    let operands = signatures.argument_tokens(source, syntax);
    let mut result = vec![];
    for command in &syntax.commands {
        if operands.contains(&command.span.start) {
            continue;
        }
        let mut reader = Reader {
            source,
            at: command.span.end,
            fields: vec![],
        };
        let parsed = (|| -> Result<Vec<Span<usize>>, ()> {
            let mut models = vec![];
            match command.name.as_str() {
                "definecolor" | "providecolor" | "xdefinecolor" => {
                    reader.class()?;
                    if !literal_name(&reader.group()?.value) {
                        return Err(());
                    }
                    if reader
                        .optional()?
                        .is_some_and(|a| !a.value.is_empty() && !syntax::is_static_key(&a.value))
                    {
                        return Err(());
                    }
                    models.push(reader.group()?.span);
                    value(&mut reader)?;
                }
                "color" | "pagecolor" | "textcolor" | "colorbox" | "fcolorbox" => {
                    if let Some(model) = reader.optional()? {
                        models.push(model.span);
                    }
                    value(&mut reader)?;
                    if command.name == "fcolorbox" {
                        if let Some(model) = reader.optional()? {
                            models.push(model.span);
                        }
                        value(&mut reader)?;
                    }
                    if matches!(
                        command.name.as_str(),
                        "textcolor" | "colorbox" | "fcolorbox"
                    ) {
                        reader.group()?;
                    }
                }
                _ => return Err(()),
            }
            if models
                .iter()
                .any(|span| !literal_name(&source[span.clone()]))
            {
                return Err(());
            }
            let mut names = BTreeSet::new();
            if models
                .iter()
                .any(|span| !names.insert(&source[span.clone()]))
            {
                return Err(());
            }
            Ok(models)
        })();
        if let Ok(models) = parsed {
            // A background model may follow a long frame specification. The
            // selected model and remaining consumed operands must survive the
            // echo, while the full raw line still has to identify it uniquely.
            result.extend(models.into_iter().map(|span| {
                let tail = span.start - 1..reader.at;
                (span, tail)
            }));
        }
    }
    result
}

pub(crate) fn index(source: &str, syntax: &mut Syntax) {
    for command in syntax
        .commands
        .iter()
        .filter(|c| known(&c.name) && !syntax.control_operands.contains(&c.span.start))
    {
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
    let signatures = crate::language::signature::CommandSignatures::new(
        workspace
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
    );
    for (_, doc) in workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
    {
        if doc
            .syntax
            .definitions
            .iter()
            .chain(&doc.syntax.environments)
            .any(|e| known(&e.name) || unsupported(&e.name))
        {
            return Err(jsonrpc::Error::invalid_params(
                "Color rename cannot resolve redefined color commands",
            ));
        }
        let operands = signatures.argument_tokens(&doc.text.source, &doc.syntax);
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if known(&command.name) && operands.contains(&command.span.start) {
                return Err(jsonrpc::Error::invalid_params(
                    "Color rename cannot resolve color commands passed as macro arguments",
                ));
            }
            let unsafe_name = |name: &str| known(name) || unsupported(name);
            let indirect = crate::symbols::named_commands::spans(doc, command)
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

fn incomplete_end(source: &str, mut at: usize) -> usize {
    let bytes = source.as_bytes();
    while at < bytes.len() {
        match bytes[at] {
            b'\n' | b'\r' | b'}' => break,
            b'%' => {
                while at < bytes.len() && !matches!(bytes[at], b'\r' | b'\n') {
                    at += 1;
                }
                if bytes.get(at) == Some(&b'\r') {
                    at += 1;
                }
                if bytes.get(at) == Some(&b'\n') {
                    at += 1;
                }
            }
            b'\\' if at + 1 < bytes.len() => at += 2,
            _ => at += 1,
        }
    }
    at
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    for command in doc.syntax.commands.iter().rev().filter(|c| {
        known(&c.name)
            && c.span.end <= offset
            && !doc.syntax.control_operands.contains(&c.span.start)
    }) {
        for field in fields(source, command).0 {
            let raw_start = field.start + 1;
            let end = syntax::group(source, field.start, b'{', b'}')
                .map(|(a, _)| a.span.end)
                .unwrap_or_else(|| incomplete_end(source, raw_start));
            if offset < raw_start || offset > end {
                continue;
            }
            let tokens = Tokens::new(source, raw_start..end);
            let Some(cursor) = tokens
                .cursor(offset)
                .or_else(|| (tokens.value.is_empty() && offset == end).then_some(0))
            else {
                return Some(vec![]);
            };
            let raw_offset = offset;
            let source = tokens.value.as_str();
            let offset = cursor;
            let end = source.len();
            let mut start = 0;
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
            let range = if at == segment_end {
                doc.text.range(raw_offset, raw_offset)
            } else {
                let Some(span) = tokens.span(at..segment_end) else {
                    return Some(vec![]);
                };
                doc.text.range(span.start, span.end)
            };
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
                    .take(workspace.completion_budget())
                    .map(|(name, detail)| CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::COLOR),
                        detail: Some(detail),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
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
