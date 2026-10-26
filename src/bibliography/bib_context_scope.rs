//! Local refcontext toggles are restored by TeX groups, independently of
//! globally stored entry assignments and context fields.
use crate::{
    parser::syntax, project::workspace::Document, symbols::commands::CommandSignatures,
    symbols::named_references::Context,
};
use std::ops::Range;

const CONTROLS: &[&str] = &[
    "begin",
    "end",
    "begingroup",
    "endgroup",
    "bgroup",
    "egroup",
    "(",
    ")",
    "[",
    "]",
];
pub(crate) fn control(name: &str) -> bool {
    CONTROLS.contains(&name) || matches!(name, "refcontext" | "newrefcontext" | "endrefcontext")
}
#[derive(PartialEq)]
enum Group {
    Simple,
    SemiSimple,
    Environment(String),
}
#[derive(Default)]
pub(crate) struct Scope {
    pub active: bool,
    frames: Vec<(Group, bool)>,
    math: Option<u8>,
}
impl Scope {
    fn push(&mut self, group: Group) -> Option<()> {
        if self.frames.len() >= 128 {
            return None;
        }
        self.frames.push((group, self.active));
        Some(())
    }
    fn pop(&mut self, group: Group) -> Option<()> {
        let (expected, active) = self.frames.pop()?;
        if group != expected {
            return None;
        }
        self.active = active;
        Some(())
    }
    fn math(&mut self, mode: u8, opening: Option<bool>) -> Option<()> {
        match (self.math, opening) {
            (None, None | Some(true)) => self.math = Some(mode),
            (Some(current), None | Some(false)) if current == mode => self.math = None,
            _ => return None,
        }
        Some(())
    }
    pub fn command(&mut self, name: &str, environment: Option<&str>) -> Option<()> {
        match name {
            "refcontext" | "newrefcontext" | "endrefcontext" => {
                // Math itself adds a scope. Until its full execution semantics
                // are certified, permit formulae but no context mutations there.
                if self.math.is_some() {
                    return None;
                }
                if name == "refcontext" && self.active {
                    return None;
                }
                self.active = name != "endrefcontext";
            }
            "begin" | "end" => {
                let environment = environment?;
                // The parser consumes the entire body and closing marker.
                // No executable toggle can occur inside native literal bodies.
                if syntax::is_verbatim_environment(environment) {
                    return Some(());
                }
                if name == "begin" {
                    self.push(Group::Environment(environment.into()))?;
                    if environment == "refcontext" {
                        if self.math.is_some() || self.active {
                            return None;
                        }
                        self.active = true;
                    }
                } else {
                    if environment == "refcontext" {
                        if self.math.is_some() {
                            return None;
                        }
                        self.active = false;
                    }
                    self.pop(Group::Environment(environment.into()))?;
                }
            }
            "begingroup" => self.push(Group::SemiSimple)?,
            "endgroup" => self.pop(Group::SemiSimple)?,
            "bgroup" => self.push(Group::Simple)?,
            "egroup" => self.pop(Group::Simple)?,
            "(" => self.math(3, Some(true))?,
            ")" => self.math(3, Some(false))?,
            "[" => self.math(4, Some(true))?,
            "]" => self.math(4, Some(false))?,
            _ => {}
        }
        Some(())
    }
    pub fn complete(&self) -> bool {
        self.frames.is_empty() && self.math.is_none()
    }

    pub fn global(&self) -> bool {
        self.complete()
    }
}

pub(crate) struct Scan<'a, 'b> {
    doc: &'a Document,
    signatures: &'b CommandSignatures<'b>,
    at: usize,
    source_end: usize,
    opaque: Vec<Range<usize>>,
    next: usize,
}
impl<'a, 'b> Scan<'a, 'b> {
    pub fn new(
        doc: &'a Document,
        signatures: &'b CommandSignatures<'b>,
        work: &mut usize,
    ) -> Option<Self> {
        let source = &doc.text.source;
        *work += source.len() + doc.syntax.commands.len();
        if *work > 32 * 1024 * 1024
            || CONTROLS
                .iter()
                .any(|name| !signatures.command_unchanged(name))
        {
            return None;
        }
        let definitions = crate::symbols::commands::definition_data_spans(source, &doc.syntax);
        for span in &definitions {
            *work += span.len();
            if *work > 32 * 1024 * 1024 {
                return None;
            }
            let mut at = span.start;
            while at < span.end {
                match source.as_bytes()[at] {
                    b'\\' => {
                        let token = syntax::token(source, at)?;
                        if matches!(token, r"\(" | r"\)" | r"\[" | r"\]") {
                            return None;
                        }
                        at += token.len();
                    }
                    b'%' => at = syntax::skip_space(source, at),
                    b'$' => return None,
                    _ => at += source[at..].chars().next()?.len_utf8(),
                }
            }
        }
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let mut source_end = source.len();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if command.span.start >= source_end {
                break;
            }
            let (name, _) = signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &Default::default(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            let canonical = signatures.forwarded_builtin(name).unwrap_or(name);
            if command.name == "endinput" || name == "endinput" || canonical == "endinput" {
                if context.literal(command.span.start) {
                    continue;
                }
                *work += source.len() + doc.syntax.commands.len();
                if *work > 32 * 1024 * 1024
                    || command.name != "endinput"
                    || name != "endinput"
                    || !signatures.command_unchanged("endinput")
                    || !positions.contains(&command.span.start)
                    || context.operand(doc, command.span.start)
                    || signatures.resource_is_tokenized(source, &doc.syntax, command.span.start)
                {
                    return None;
                }
                // TeX finishes the current line, then stops reading this input.
                source_end = source[command.span.end..]
                    .find(['\r', '\n'])
                    .map_or(source.len(), |after| command.span.end + after);
                continue;
            }
            if !CONTROLS.contains(&canonical) {
                continue;
            }
            *work += source.len() + doc.syntax.commands.len();
            if *work > 32 * 1024 * 1024 {
                return None;
            }
            // Group wrappers can restore the toggle behind the textual trace.
            // Certify native executable controls; retain all other cases as guards.
            if command.name != canonical
                || name != canonical
                || !positions.contains(&command.span.start)
                || context.operand(doc, command.span.start)
                || signatures.resource_is_tokenized(source, &doc.syntax, command.span.start)
            {
                return None;
            }
        }
        let mut opaque: Vec<_> = doc
            .syntax
            .ignored
            .iter()
            .chain(&doc.syntax.literal_conversions)
            .chain(&doc.syntax.literal_tests)
            .chain(&doc.syntax.ambiguous_string_tests)
            .cloned()
            .collect();
        opaque.extend(definitions);
        opaque.extend(crate::symbols::expl3::math_data_spans(doc));
        opaque.sort_unstable_by_key(|span| span.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for span in opaque {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| span.start <= previous.end)
            {
                previous.end = previous.end.max(span.end);
            } else {
                merged.push(span);
            }
        }
        Some(Self {
            doc,
            signatures,
            at: 0,
            source_end,
            opaque: merged,
            next: 0,
        })
    }
    pub fn skip_command(&mut self, end: usize) {
        self.at = self.at.max(end);
    }
    pub fn advance(&mut self, end: usize, scope: &mut Scope, work: &mut usize) -> Option<()> {
        let source = &self.doc.text.source;
        let end = end.min(self.source_end);
        while self.at < end {
            while self
                .opaque
                .get(self.next)
                .is_some_and(|span| span.end <= self.at)
            {
                self.next += 1;
            }
            if let Some(span) = self
                .opaque
                .get(self.next)
                .filter(|span| span.start <= self.at)
            {
                self.at = span.end;
                continue;
            }
            if self.doc.syntax.punctuation_operands.contains(&self.at) {
                self.at += 1;
                continue;
            }
            match source.as_bytes()[self.at] {
                b'\\' => {
                    let token = syntax::token(source, self.at)?;
                    if matches!(token, r"\(" | r"\)" | r"\[" | r"\]") {
                        *work += self.doc.syntax.commands.len().max(1);
                        if *work > 32 * 1024 * 1024 {
                            return None;
                        }
                        if !self.doc.syntax.control_operands.contains(&self.at)
                            && !self.signatures.known_literal_is_argument(
                                source,
                                &self.doc.syntax,
                                self.at,
                            )
                        {
                            scope.math(
                                if matches!(token, r"\(" | r"\)") { 3 } else { 4 },
                                Some(matches!(token, r"\(" | r"\[")),
                            )?;
                        }
                    }
                    self.at += token.len();
                }
                b'{' => {
                    scope.push(Group::Simple)?;
                    self.at += 1;
                }
                b'}' => {
                    scope.pop(Group::Simple)?;
                    self.at += 1;
                }
                b'$' => {
                    *work += self.doc.syntax.commands.len().max(1);
                    if *work > 32 * 1024 * 1024 {
                        return None;
                    }
                    if self
                        .signatures
                        .known_literal_is_argument(source, &self.doc.syntax, self.at)
                    {
                        self.at += 1;
                        continue;
                    }
                    let width = if source.as_bytes().get(self.at + 1) == Some(&b'$') {
                        2
                    } else {
                        1
                    };
                    scope.math(width as u8, None)?;
                    self.at += width;
                }
                _ => self.at += source[self.at..].chars().next()?.len_utf8(),
            }
        }
        Some(())
    }
}
