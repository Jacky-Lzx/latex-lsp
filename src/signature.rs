//! Linearly match call arguments; do not execute macro definitions.
use crate::{
    catalog::COMMANDS,
    syntax::{self, Entry},
    workspace::{Document, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

#[derive(Clone)]
enum Kind {
    Required,
    RequiredGroup,
    CommandToken,
    RequiredBracket,
    Optional,
    UnstarredOptional,
    Star,
    Flag(String),
    Delimited {
        open: String,
        close: String,
        optional: bool,
    },
}

struct Parameter {
    obey_spaces: bool,
    kind: Kind,
    label: String,
}

struct Signature {
    name: String,
    parameters: Vec<Parameter>,
    documentation: String,
    starred: bool,
    nested_brackets: bool,
}

impl Signature {
    fn label(&self) -> String {
        format!(
            "\\{}{}{}",
            self.name,
            if self
                .parameters
                .first()
                .is_some_and(|parameter| starts_letter(&parameter.label))
            {
                " "
            } else {
                ""
            },
            self.parameters
                .iter()
                .map(|parameter| parameter.label.as_str())
                .collect::<String>()
        )
    }

    fn information(self, active: usize) -> SignatureHelp {
        let label = self.label();
        SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: Some(Documentation::String(self.documentation)),
                parameters: Some(
                    self.parameters
                        .into_iter()
                        .map(|parameter| ParameterInformation {
                            label: ParameterLabel::Simple(parameter.label),
                            documentation: parameter.obey_spaces.then(|| Documentation::String(
                                "An intervening TeX space prevents this optional argument from being read (xparse !).".into(),
                            )),
                        })
                        .collect(),
                ),
                active_parameter: Some(active as u32),
            }],
            active_signature: Some(0),
            active_parameter: Some(active as u32),
        }
    }
}

/// Omit optional arguments and flags so accepting a completion preserves the
/// macro's defaults. Tabstops follow the order of its required arguments.
pub fn completion(
    workspace: &Workspace,
    name: &str,
    scope: &BTreeSet<Url>,
) -> Option<(String, Option<String>)> {
    completion_at(workspace, name, scope, "")
}

pub(crate) fn completion_at(
    workspace: &Workspace,
    name: &str,
    scope: &BTreeSet<Url>,
    following: &str,
) -> Option<(String, Option<String>)> {
    let signature = signature(workspace, name, scope)?;
    if existing_arguments(&signature, following) {
        return Some((signature.label(), None));
    }
    let mut snippet = name.to_owned();
    let mut tabstop = 0;
    for (index, parameter) in signature.parameters.iter().enumerate() {
        let (open, close) = match &parameter.kind {
            Kind::Required | Kind::RequiredGroup | Kind::CommandToken => ("{", "}"),
            Kind::RequiredBracket => ("[", "]"),
            Kind::Delimited {
                open,
                close,
                optional: false,
            } => (open.as_str(), close.as_str()),
            _ => continue,
        };
        let number = parameter
            .label
            .strip_prefix("{#")
            .and_then(|label| label.strip_suffix('}'))
            .and_then(|number| number.parse::<usize>().ok())
            .unwrap_or(index + 1);
        tabstop += 1;
        if tabstop == 1 && starts_letter(open) {
            snippet.push(' ');
        }
        if matches!(parameter.kind, Kind::Delimited { .. }) {
            push_delimiter(&mut snippet, open);
        } else {
            snippet.push_str(open);
        }
        snippet.push_str(&format!("${{{tabstop}:arg{number}}}"));
        if matches!(parameter.kind, Kind::Delimited { .. }) {
            push_delimiter(&mut snippet, close);
        } else {
            snippet.push_str(close);
        }
    }
    Some((signature.label(), (tabstop > 0).then_some(snippet)))
}

pub(crate) fn has_existing_arguments(
    workspace: &Workspace,
    name: &str,
    scope: &BTreeSet<Url>,
    following: &str,
) -> bool {
    signature(workspace, name, scope)
        .is_some_and(|signature| existing_arguments(&signature, following))
}

fn existing_arguments(signature: &Signature, following: &str) -> bool {
    let ungrouped_argument = completion_required_token(following);
    let following = &following[syntax::skip_space(following, 0)..];
    // The same argument reader decides whether existing custom delimiters must
    // be preserved. Stop at the first required argument: later delimiters cannot
    // start a valid call here.
    for parameter in &signature.parameters {
        match &parameter.kind {
            Kind::Delimited { open, optional, .. } => {
                if token(following, 0) == Some(open.as_str()) {
                    return true;
                }
                if !optional {
                    break;
                }
            }
            Kind::Optional | Kind::UnstarredOptional | Kind::Star => {}
            Kind::Flag(flag) => {
                if token(following, 0) == Some(flag.as_str()) {
                    return true;
                }
            }
            Kind::Required if ungrouped_argument.is_some() => return true,
            Kind::CommandToken
                if ungrouped_argument
                    .is_some_and(|value| syntax::control_word(value, 0).is_some()) =>
            {
                return true;
            }
            _ => break,
        }
    }
    false
}

fn starts_letter(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '@')
}

fn push_delimiter(snippet: &mut String, delimiter: &str) {
    for ch in delimiter.chars() {
        if matches!(ch, '\\' | '$' | '}') {
            snippet.push('\\');
        }
        snippet.push(ch);
    }
    // A control word followed by an argument placeholder (or another letter
    // delimiter) needs a separator; TeX discards this space when tokenizing.
    if delimiter.starts_with('\\') && syntax::control_word(delimiter, 0).is_some() {
        snippet.push(' ');
    }
}

fn builtin(name: &str) -> Option<Signature> {
    let (arguments, starred): (&[(&str, bool)], bool) = match name {
        name if syntax::declares_traditional_command(name) => (
            &[
                ("command", false),
                ("arguments", true),
                ("default", true),
                ("definition", false),
            ],
            true,
        ),
        "newenvironment" | "renewenvironment" => (
            &[
                ("environment", false),
                ("arguments", true),
                ("default", true),
                ("begin", false),
                ("end", false),
            ],
            true,
        ),
        "DeclareTextCommand" | "ProvideTextCommand" => (
            &[
                ("command", false),
                ("encoding", false),
                ("arguments", true),
                ("default", true),
                ("definition", false),
            ],
            false,
        ),
        "DeclareTextCommandDefault" | "ProvideTextCommandDefault" => (
            &[
                ("command", false),
                ("arguments", true),
                ("default", true),
                ("definition", false),
            ],
            false,
        ),
        "DeclareTextSymbol" | "DeclareTextAccent" => (
            &[("command", false), ("encoding", false), ("slot", false)],
            false,
        ),
        name if syntax::declares_environment_copy(name) => (
            &[("environment", false), ("source environment", false)],
            false,
        ),
        name if syntax::declares_command_copy(name) => {
            (&[("command", false), ("source command", false)], false)
        }
        name if syntax::declares_document_command(name) => (
            &[
                ("command", false),
                ("argument specification", false),
                ("definition", false),
            ],
            false,
        ),
        "DeclareMathOperator" => (&[("command", false), ("text", false)], true),
        "operatorname" => (&[("text", false)], true),
        "frac" | "dfrac" | "tfrac" | "binom" => {
            (&[("numerator", false), ("denominator", false)], false)
        }
        "sqrt" => (&[("degree", true), ("expression", false)], false),
        "hat" | "widehat" | "tilde" | "widetilde" | "bar" | "vec" | "dot" | "ddot" | "acute"
        | "grave" | "breve" | "check" | "mathring" | "overline" | "underline" | "sqrtsign" => {
            (&[("expression", false)], false)
        }
        "href" => (&[("url", false), ("text", false)], false),
        "url" => (&[("url", false)], false),
        "definecolor" | "providecolor" | "xdefinecolor" => (
            &[
                ("type", true),
                ("name", false),
                ("prefix", true),
                ("model", false),
                ("specification", false),
            ],
            false,
        ),
        "colorlet" => (
            &[
                ("type", true),
                ("name", false),
                ("model", true),
                ("color", false),
            ],
            false,
        ),
        "color" | "pagecolor" => (&[("model", true), ("color", false)], false),
        "rowcolor" | "columncolor" => (
            &[
                ("model", true),
                ("color", false),
                ("left overhang", true),
                ("right overhang", true),
            ],
            false,
        ),
        "cellcolor" | "arrayrulecolor" | "doublerulesepcolor" | "maskcolors" => {
            (&[("model", true), ("color", false)], false)
        }
        "blendcolors" => (&[("color expression", false)], true),
        "textcolor" | "colorbox" => (&[("model", true), ("color", false), ("text", false)], false),
        "fcolorbox" => (
            &[
                ("frame model", true),
                ("frame color", false),
                ("background model", true),
                ("background color", false),
                ("text", false),
            ],
            false,
        ),
        "includegraphics" => (&[("options", true), ("file", false)], true),
        name if crate::fancyvrb::is_input(name) => (&[("options", true), ("file", false)], true),
        "verbatiminput" => (&[("file", false)], true),
        "lstinputlisting" => (&[("options", true), ("file", false)], false),
        "documentclass" | "usepackage" | "RequirePackage" | "LoadClass" => (
            &[
                ("options", true),
                ("name", false),
                ("minimum version", true),
            ],
            false,
        ),
        "ProvidesPackage" | "ProvidesClass" | "ProvidesFile" | "NeedsTeXFormat" => {
            (&[("name", false), ("release information", true)], false)
        }
        name if syntax::loader_forwards_options(name) => {
            (&[("name", false), ("minimum version", true)], false)
        }
        "input" | "include" | "subfile" | "bibliography" | "bibliographystyle" => {
            (&[("file", false)], false)
        }
        "InputIfFileExists" => (
            &[
                ("file", false),
                ("before input", false),
                ("if missing", false),
            ],
            false,
        ),
        "IfFileExists" => (
            &[("file", false), ("if exists", false), ("if missing", false)],
            false,
        ),
        "PassOptionsToPackage" | "PassOptionsToClass" => {
            (&[("options", false), ("name", false)], false)
        }
        "includeonly" => (&[("files", false)], false),
        "csuse" | "csmeaning" | "csundef" | "csgundef" => (&[("command", false)], false),
        "cslet" | "letcs" | "csletcs" => (&[("alias", false), ("command", false)], false),
        "csappto" | "cseappto" | "csgappto" | "csxappto" | "cspreto" | "csepreto" | "csgpreto"
        | "csxpreto" => (&[("command", false), ("code", false)], false),
        "ifcsdef" | "ifcsundef" | "ifcsvoid" | "ifcsmacro" | "ifcsparam" | "ifcsprotected"
        | "ifcsltxprotect" => (
            &[("command", false), ("then", false), ("else", false)],
            false,
        ),
        "ifcsstring" => (
            &[
                ("command", false),
                ("string", false),
                ("then", false),
                ("else", false),
            ],
            false,
        ),
        "ifcsequal" | "ifcsstrequal" => (
            &[
                ("first command", false),
                ("second command", false),
                ("then", false),
                ("else", false),
            ],
            false,
        ),
        "addbibresource" => (&[("options", true), ("file", false)], false),
        "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
        | "subparagraph" | "caption" => (&[("short title", true), ("title", false)], true),
        "footnote" => (&[("number", true), ("text", false)], false),
        "textbf" | "textit" | "texttt" | "emph" | "title" | "author" | "date" => {
            (&[("text", false)], false)
        }
        "typeout" | "message" => (&[("text", false)], false),
        "extractcolorspec" => (&[("color", false), ("specification command", false)], false),
        "extractcolorspecs" => (
            &[
                ("color", false),
                ("model command", false),
                ("specification command", false),
            ],
            false,
        ),
        "convertcolorspec" => (
            &[
                ("source model", false),
                ("color specification", false),
                ("target model", false),
                ("output command", false),
            ],
            false,
        ),
        "begin" | "end" => (&[("environment", false)], false),
        "label" => (&[("key", false)], false),
        "hyperref" => (&[("key", true), ("text", false)], false),
        "hypertarget" | "hyperlink" => (&[("target", false), ("text", false)], false),
        "pdfbookmark" => (&[("level", true), ("title", false), ("name", false)], false),
        "hyperdef" => (
            &[
                ("key", true),
                ("category", false),
                ("name", false),
                ("text", false),
            ],
            false,
        ),
        "newcounter" => (&[("name", false), ("within", true)], false),
        "newboolean" | "provideboolean" | "newbool" | "providebool" | "boolean" | "booltrue"
        | "boolfalse" => (&[("boolean", false)], false),
        "setboolean" | "setbool" => (&[("boolean", false), ("value", false)], false),
        "newtoggle" | "providetoggle" | "toggletrue" | "togglefalse" => {
            (&[("toggle", false)], false)
        }
        "settoggle" => (&[("toggle", false), ("value", false)], false),
        "iftoggle" | "nottoggle" => (
            &[("toggle", false), ("then", false), ("else", false)],
            false,
        ),
        "ifbool" | "notbool" => (
            &[("boolean", false), ("then", false), ("else", false)],
            false,
        ),
        "ifthenelse" => (
            &[("condition", false), ("then", false), ("else", false)],
            false,
        ),
        "ifboolexpr" | "ifboolexpe" => (
            &[("condition", false), ("then", false), ("else", false)],
            false,
        ),
        "whileboolexpr" | "unlessboolexpr" => (&[("condition", false), ("body", false)], false),
        "newlength" => (&[("length", false)], false),
        "setlength" | "addtolength" => (&[("length", false), ("value", false)], false),
        "settowidth" | "settoheight" | "settodepth" => {
            (&[("length", false), ("text", false)], false)
        }
        "newsavebox" | "usebox" => (&[("box", false)], false),
        "sbox" => (&[("box", false), ("text", false)], false),
        "savebox" => (
            &[
                ("box", false),
                ("width", true),
                ("position", true),
                ("text", false),
            ],
            false,
        ),
        "newfont" => (&[("command", false), ("font", false)], false),
        "DeclareTextFontCommand" => (&[("command", false), ("font commands", false)], false),
        "DeclareFixedFont" => (
            &[
                ("command", false),
                ("encoding", false),
                ("family", false),
                ("series", false),
                ("shape", false),
                ("size", false),
            ],
            false,
        ),
        "DeclareMathAlphabet" => (
            &[
                ("command", false),
                ("encoding", false),
                ("family", false),
                ("series", false),
                ("shape", false),
            ],
            false,
        ),
        "DeclareSymbolFontAlphabet" => (&[("command", false), ("symbol font", false)], false),
        "DeclareMathSymbol" | "DeclareMathAccent" => (
            &[
                ("command", false),
                ("class", false),
                ("symbol font", false),
                ("slot", false),
            ],
            false,
        ),
        "DeclareMathRadical" => (
            &[
                ("command", false),
                ("small symbol font", false),
                ("small slot", false),
                ("large symbol font", false),
                ("large slot", false),
            ],
            false,
        ),
        "DeclareMathDelimiter" => (
            &[
                ("command", false),
                ("class", false),
                ("small symbol font", false),
                ("small slot", false),
                ("large symbol font", false),
                ("large slot", false),
            ],
            false,
        ),
        "DeclarePairedDelimiter" => (
            &[("command", false), ("left", false), ("right", false)],
            false,
        ),
        "DeclarePairedDelimiterX" => (
            &[
                ("command", false),
                ("arguments", true),
                ("left", false),
                ("right", false),
                ("body", false),
            ],
            false,
        ),
        "DeclarePairedDelimiterXPP" => (
            &[
                ("command", false),
                ("arguments", true),
                ("before", false),
                ("left", false),
                ("right", false),
                ("after", false),
                ("body", false),
            ],
            false,
        ),
        name if syntax::declares_font_switch(name) => (
            &[
                ("command", false),
                ("features", true),
                ("font", false),
                ("features", true),
            ],
            false,
        ),
        "newaliascnt" => (&[("alias", false), ("counter", false)], false),
        "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref" => {
            (&[("counter", false), ("key", false)], false)
        }
        "setcounter" | "addtocounter" => (&[("counter", false), ("value", false)], false),
        "stepcounter" | "refstepcounter" | "usecounter" | "value" | "arabic" | "roman"
        | "Roman" | "alph" | "Alph" | "fnsymbol" | "aliascntresetthe" => {
            (&[("counter", false)], false)
        }
        "counterwithin" | "counterwithout" => (
            &[("format", true), ("counter", false), ("within", false)],
            true,
        ),
        "numberwithin" => (
            &[("format", true), ("counter", false), ("within", false)],
            false,
        ),
        "getrefnumber" | "getpagerefnumber" | "refused" => (&[("key", false)], false),
        "getrefbykeydefault" => (
            &[("key", false), ("field", false), ("default", false)],
            false,
        ),
        "IfRefUndefinedExpandable" | "IfRefUndefinedBabel" => (
            &[("key", false), ("undefined", false), ("defined", false)],
            false,
        ),
        "vref" | "Vref" => (&[("same-page", true), ("key", false)], true),
        "vpageref" => (
            &[("same-page", true), ("prefix", true), ("key", false)],
            true,
        ),
        "fullref" => (&[("key", false)], false),
        "vrefrange" | "vpagerefrange" => (
            &[("same-page", true), ("first", false), ("last", false)],
            syntax::reference_accepts_star(name),
        ),
        "crefrange" | "Crefrange" | "cpagerefrange" | "Cpagerefrange" => {
            (&[("first", false), ("last", false)], true)
        }
        name if syntax::is_reference(name) => {
            (&[("keys", false)], syntax::reference_accepts_star(name))
        }
        "bibitem" => (&[("label", true), ("key", false)], false),
        "bibentry" => (&[("key", false)], false),
        "nobibliography" => (&[("databases", false)], false),
        "defcitealias" => (&[("key", false), ("text", false)], false),
        "citenum" => (&[("keys", false)], false),
        "citeyearpar" | "citefullauthor" | "citetalias" | "citepalias" => (
            &[
                ("note / prenote", true),
                ("postnote", true),
                ("keys", false),
            ],
            false,
        ),
        name if syntax::is_citation(name) && name != "nocite" => (
            &[
                ("note / prenote", true),
                ("postnote", true),
                ("keys", false),
            ],
            syntax::citation_accepts_star(name),
        ),
        "nocite" => (&[("keys", false)], false),
        _ => return None,
    };
    let parameters = arguments
        .iter()
        .enumerate()
        .map(|(index, (label, optional))| Parameter {
            obey_spaces: false,
            kind: if index == 1
                && matches!(
                    name,
                    "DeclarePairedDelimiterX" | "DeclarePairedDelimiterXPP"
                ) {
                Kind::RequiredBracket
            } else if *optional {
                if matches!(
                    name,
                    "part"
                        | "chapter"
                        | "section"
                        | "subsection"
                        | "subsubsection"
                        | "paragraph"
                        | "subparagraph"
                ) {
                    Kind::UnstarredOptional
                } else {
                    Kind::Optional
                }
            } else if syntax::declares_command_copy(name)
                || name == "cslet" && index == 1
                || (name == "letcs"
                    || syntax::declares_traditional_command(name)
                    || syntax::declares_document_command(name)
                    || syntax::declares_text_command(name)
                    || syntax::declares_font_switch(name)
                    || syntax::declares_paired_delimiter(name)
                    || matches!(
                        name,
                        "DeclareMathOperator"
                            | "DeclareTextFontCommand"
                            | "DeclareFixedFont"
                            | "DeclareMathAlphabet"
                            | "DeclareSymbolFontAlphabet"
                            | "DeclareMathSymbol"
                            | "DeclareMathAccent"
                            | "DeclareMathRadical"
                            | "DeclareMathDelimiter"
                    ))
                    && index == 0
            {
                Kind::CommandToken
            } else if name == "verbatiminput"
                || syntax::is_range_reference(name)
                || index == 1
                    && matches!(
                        name,
                        "definecolor"
                            | "providecolor"
                            | "xdefinecolor"
                            | "colorlet"
                            | "textcolor"
                            | "colorbox"
                    )
                || name == "fcolorbox" && matches!(index, 1 | 3)
            {
                // Keep range endpoints aligned with reference navigation and
                // color names aligned with their grouped declaration/wrappers.
                // Unsupported stars and extra notes are not bare names.
                Kind::RequiredGroup
            } else {
                Kind::Required
            },
            label: if *optional {
                format!("[{label}]")
            } else {
                format!("{{{label}}}")
            },
        })
        .collect();
    let documentation = COMMANDS
        .iter()
        .find(|(command, _, _)| *command == name)
        .map(|(_, _, documentation)| *documentation)
        .unwrap_or("LaTeX command arguments.")
        .into();
    Some(Signature {
        name: name.into(),
        parameters,
        documentation,
        starred,
        nested_brackets: !(name == "lstinputlisting"
            || crate::fancyvrb::is_input(name)
            || syntax::is_citation(name)
            || syntax::loader_extension(name).is_some()
            || syntax::declares_traditional_command(name)
            || syntax::reference_note_count(name) > 0
            || matches!(
                name,
                "hyperref"
                    | "definecolor"
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
                    | "maskcolors"
                    | "ProvidesPackage"
                    | "ProvidesClass"
                    | "ProvidesFile"
                    | "NeedsTeXFormat"
                    | "bibitem"
                    | "newenvironment"
                    | "renewenvironment"
                    | "hyperdef"
                    | "newcounter"
                    | "sqrt"
                    | "part"
                    | "chapter"
                    | "section"
                    | "subsection"
                    | "subsubsection"
                    | "paragraph"
                    | "subparagraph"
                    | "caption"
            )),
    })
}

fn definition(uri: &Url, doc: &Document, entry: &Entry, environment: bool) -> Option<Signature> {
    let mut signature = source_definition(&doc.text.source, &doc.syntax, entry, environment)?;
    if signature.parameters.is_empty() || environment && signature.parameters.len() == 1 {
        return None;
    }
    signature.documentation = format!(
        "{}\n{}:{}",
        entry.detail,
        uri.path(),
        doc.text.position(entry.span.start).line + 1
    );
    Some(signature)
}

fn source_definition(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
    environment: bool,
) -> Option<Signature> {
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let mut i = syntax::skip_space(source, entry.span.end);
    if !environment
        && !crate::named_commands::declares(declaration)
        && source.as_bytes().get(entry.span.start) != Some(&b'\\')
    {
        let end = parsed
            .commands
            .iter()
            .find(|command| command.span.start == i && command.name == "endcsname")?;
        i = syntax::skip_space(source, end.span.end);
    }
    if source.as_bytes().get(i) == Some(&b'}') {
        i += 1;
    }
    i = syntax::skip_space(source, i);
    if matches!(declaration, "DeclareTextCommand" | "ProvideTextCommand") {
        let (_, end) = syntax::group(source, i, b'{', b'}')?;
        i = syntax::skip_space(source, end);
    }
    let mut parameters = Vec::new();
    let mut nested_brackets = true;
    match declaration {
        "DeclareTextFontCommand"
        | "DeclareMathAlphabet"
        | "DeclareSymbolFontAlphabet"
        | "DeclareTextAccent"
        | "DeclareMathAccent"
        | "DeclareMathRadical" => {
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::Required,
                label: "{#1}".into(),
            });
        }
        declaration if syntax::declares_paired_delimiter(declaration) => {
            let count = if declaration == "DeclarePairedDelimiter" {
                1
            } else {
                let (argument, _) = syntax::group(source, i, b'[', b']')?;
                let count = argument.value.trim().parse::<usize>().ok()?;
                if !(1..=9).contains(&count) {
                    return None;
                }
                count
            };
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::Star,
                label: "*".into(),
            });
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::UnstarredOptional,
                label: "[size]".into(),
            });
            for number in 1..=count {
                parameters.push(Parameter {
                    obey_spaces: false,
                    kind: Kind::Required,
                    label: format!("{{#{number}}}"),
                });
            }
        }
        "newcommand"
        | "DeclareTextCommand"
        | "ProvideTextCommand"
        | "DeclareTextCommandDefault"
        | "ProvideTextCommandDefault"
        | "renewcommand"
        | "providecommand"
        | "newrobustcmd"
        | "renewrobustcmd"
        | "providerobustcmd"
        | "DeclareRobustCommand"
        | "newenvironment"
        | "renewenvironment"
        | "provideenvironment"
        | "NewEnviron"
        | "RenewEnviron" => {
            nested_brackets = false;
            // A traditional declaration without a count has no arguments.
            // Keep this information for operand analysis; signature help
            // still omits empty signatures.
            if source.as_bytes().get(i) == Some(&b'{') {
                return Some(Signature {
                    name: entry.name.clone(),
                    parameters,
                    documentation: entry.detail.clone(),
                    starred: false,
                    nested_brackets,
                });
            }
            let (count, end) = syntax::group(source, i, b'[', b']')?;
            let count: usize = count.value.trim().parse().ok()?;
            if !(1..=9).contains(&count) {
                return None;
            }
            i = syntax::skip_space(source, end);
            let default = syntax::bracket_argument(source, i).map(|(argument, _)| argument.value);
            for number in 1..=count {
                let optional = number == 1 && default.is_some();
                let label = if optional {
                    format!("[#1={}]", short(default.as_deref().unwrap()))
                } else {
                    format!("{{#{number}}}")
                };
                parameters.push(Parameter {
                    obey_spaces: false,
                    kind: if optional {
                        Kind::Optional
                    } else {
                        Kind::Required
                    },
                    label,
                });
            }
        }
        declaration
            if syntax::declares_document_command(declaration)
                || matches!(
                    declaration,
                    "NewDocumentEnvironment"
                        | "RenewDocumentEnvironment"
                        | "ProvideDocumentEnvironment"
                        | "DeclareDocumentEnvironment"
                ) =>
        {
            let (arguments, _) = syntax::group(source, i, b'{', b'}')?;
            let source = &arguments.value;
            let mut i = 0;
            let mut obey_spaces = false;
            let mut seen_obey_spaces = false;
            while i < source.len() {
                i = syntax::skip_space(source, i);
                let Some(&kind) = source.as_bytes().get(i) else {
                    break;
                };
                i += 1;
                if kind == b'+' {
                    continue;
                }
                if kind == b'!' {
                    if obey_spaces || declaration.contains("ExpandableDocumentCommand") {
                        return None;
                    }
                    obey_spaces = true;
                    seen_obey_spaces = true;
                    continue;
                }
                // The kernel permits ! only on the final optional arguments.
                // A mandatory argument after any such modifier is invalid.
                if seen_obey_spaces && matches!(kind, b'm' | b'r' | b'R') {
                    return None;
                }
                let number = parameters.len() + 1;
                let (kind, label) = match kind {
                    b'm' => (Kind::Required, format!("{{#{number}}}")),
                    b'o' => (Kind::Optional, format!("[#{number}]")),
                    b'O' => {
                        i = syntax::skip_space(source, i);
                        let (default, end) = syntax::group(source, i, b'{', b'}')?;
                        i = end;
                        (
                            Kind::Optional,
                            format!("[#{number}={}]", short(&default.value)),
                        )
                    }
                    b's' => (Kind::Star, "*".into()),
                    b't' => {
                        let (flag, end) = delimiter_spec(source, i)?;
                        i = end;
                        (Kind::Flag(flag.clone()), flag)
                    }
                    b'd' | b'D' | b'r' | b'R' => {
                        let (open, end) = delimiter_spec(source, i)?;
                        let (close, end) = delimiter_spec(source, end)?;
                        i = end;
                        let default = if matches!(kind, b'D' | b'R') {
                            i = syntax::skip_space(source, i);
                            let (default, end) = syntax::group(source, i, b'{', b'}')?;
                            i = end;
                            format!("={}", short(&default.value))
                        } else {
                            String::new()
                        };
                        let label = format!("{open}#{number}{default}{close}");
                        (
                            Kind::Delimited {
                                open,
                                close,
                                optional: matches!(kind, b'd' | b'D'),
                            },
                            label,
                        )
                    }
                    _ => return None,
                };
                parameters.push(Parameter {
                    kind,
                    label,
                    obey_spaces,
                });
                obey_spaces = false;
                if parameters.len() > 9 {
                    return None;
                }
            }
            if obey_spaces {
                return None;
            }
        }
        "newtheorem" => {
            nested_brackets = false;
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::Optional,
                label: "[note]".into(),
            });
        }
        "def" | "gdef" | "edef" | "xdef" | "csdef" | "csgdef" | "csedef" | "csxdef" => {
            while source.as_bytes().get(i) == Some(&b'#') {
                let number = parameters.len() + 1;
                if number > 9 || source.as_bytes().get(i + 1) != Some(&(b'0' + number as u8)) {
                    return None;
                }
                parameters.push(Parameter {
                    obey_spaces: false,
                    kind: Kind::Required,
                    label: format!("{{#{number}}}"),
                });
                i += 2;
            }
            if source.as_bytes().get(i) != Some(&b'{') {
                return None;
            }
        }
        _ => return None,
    }
    if environment {
        parameters.insert(
            0,
            Parameter {
                obey_spaces: false,
                kind: Kind::Required,
                label: format!("{{{}}}", entry.name),
            },
        );
    }
    Some(Signature {
        name: if environment {
            "begin".into()
        } else {
            entry.name.clone()
        },
        parameters,
        documentation: entry.detail.clone(),
        starred: false,
        nested_brackets,
    })
}

fn short(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

/// Read one lexical TeX token without expanding it. In particular, a delimiter
/// control word must not match a prefix of a longer control word.
fn token(source: &str, start: usize) -> Option<&str> {
    let ch = source.get(start..)?.chars().next()?;
    let end = if ch == '\\' {
        syntax::control_word(source, start).map_or_else(
            || {
                source[start + 1..]
                    .chars()
                    .next()
                    .map(|ch| start + 1 + ch.len_utf8())
            },
            |span| Some(span.end),
        )?
    } else {
        start + ch.len_utf8()
    };
    Some(&source[start..end])
}

/// Only infer ungrouped tokens whose boundaries agree across the engines. In
/// pdfTeX a Unicode character consists of several UTF-8 tokens; XeTeX reads one
/// character and may include Unicode letters in a control word instead.
fn required_token(source: &str, start: usize) -> Option<&str> {
    let rest = source.get(start..)?;
    if rest.starts_with("^^") || rest.starts_with("\\^^") {
        return None;
    }
    let value = token(source, start)?;
    if !value.is_ascii()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || (value.contains(' ') && value != "\\ ")
        || matches!(value, "{" | "}" | "%" | "\\par")
    {
        return None;
    }
    if syntax::control_word(value, 0).is_some() {
        let following = &source[start + value.len()..];
        if following.starts_with("^^") || following.chars().next().is_some_and(|ch| !ch.is_ascii())
        {
            return None;
        }
    }
    Some(value)
}

/// Control words read as bare arguments are data, rather than direct calls.
/// Reuse signature grammars for local declarations and built-ins. Unknown or
/// ambiguous commands cannot certify how many leading groups they consume.
pub(crate) fn command_argument_tokens(source: &str, parsed: &syntax::Syntax) -> BTreeSet<usize> {
    CommandSignatures::new([(source, parsed)]).argument_tokens(source, parsed)
}

/// Resolve command arities once across the documents in the analyzed project.
pub(crate) struct CommandSignatures<'a> {
    local: BTreeMap<&'a str, Option<Signature>>,
}

impl<'a> CommandSignatures<'a> {
    pub(crate) fn new(documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>) -> Self {
        let mut local = BTreeMap::new();
        for (source, parsed) in documents {
            for entry in &parsed.definitions {
                let signature = source_definition(source, parsed, entry, false);
                match local.entry(entry.name.as_str()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(signature);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        entry.insert(None);
                    }
                }
            }
        }
        Self { local }
    }

    pub(crate) fn argument_tokens(&self, source: &str, parsed: &syntax::Syntax) -> BTreeSet<usize> {
        let local = &self.local;
        let mut result = BTreeSet::new();
        for command in &parsed.commands {
            if result.contains(&command.span.start) {
                continue;
            }
            let signature = local.get(command.name.as_str()).and_then(Option::as_ref);
            let builtin = (!local.contains_key(command.name.as_str()))
                .then(|| builtin(&command.name))
                .flatten();
            if let Some(signature) = signature.or(builtin.as_ref()) {
                argument_tokens(source, command.span.end, signature, &mut result);
            } else if !local.contains_key(command.name.as_str())
                && matches!(
                    command.name.as_str(),
                    "relax"
                        | "noindent"
                        | "indent"
                        | "normalfont"
                        | "bfseries"
                        | "mdseries"
                        | "itshape"
                        | "upshape"
                        | "slshape"
                        | "scshape"
                        | "rmfamily"
                        | "sffamily"
                        | "ttfamily"
                        | "begingroup"
                        | "endgroup"
                        | "bgroup"
                        | "egroup"
                        | "hline"
                        | "par"
                        | "newline"
                        | "hfill"
                        | "vfill"
                        | "hfil"
                        | "vfil"
                        | "iftrue"
                        | "iffalse"
                        | "else"
                        | "fi"
                        | "or"
                        | "tiny"
                        | "scriptsize"
                        | "footnotesize"
                        | "small"
                        | "normalsize"
                        | "large"
                        | "Large"
                        | "LARGE"
                        | "huge"
                        | "Huge"
                )
            {
                // These standard switches consume no argument. Local redefinitions
                // take precedence, including declarations we cannot interpret.
            } else {
                unknown_argument_token(source, command.span.end, &mut result);
            }
        }
        result
    }
}

fn unknown_argument_token(source: &str, mut at: usize, result: &mut BTreeSet<usize>) {
    loop {
        at = syntax::skip_space(source, at);
        let grouped = match source.as_bytes().get(at) {
            Some(b'{') => syntax::group(source, at, b'{', b'}'),
            Some(b'[') => syntax::bracket_argument(source, at),
            Some(b'*') => {
                at += 1;
                continue;
            }
            _ => None,
        };
        if let Some((_, end)) = grouped {
            at = end;
        } else {
            if syntax::control_word(source, at).is_some() {
                result.insert(at);
            }
            return;
        }
    }
}

fn operand_delimited_end(source: &str, at: usize, open: &str, close: &str) -> Option<usize> {
    let mut at = at + open.len();
    loop {
        at = syntax::skip_space(source, at);
        let value = token(source, at)?;
        if value == close {
            return Some(at + close.len());
        }
        if value == "{" {
            at = syntax::group(source, at, b'{', b'}')?.1;
        } else {
            at += value.len();
        }
    }
}

fn argument_tokens(
    source: &str,
    mut at: usize,
    signature: &Signature,
    result: &mut BTreeSet<usize>,
) {
    let mut control_spaces = true;
    let mut starred = false;
    if signature.starred {
        at = optional_start(source, at, true);
        if source.as_bytes().get(at) == Some(&b'*') {
            at += 1;
            starred = true;
            control_spaces = false;
        }
    }
    for parameter in &signature.parameters {
        if starred && matches!(parameter.kind, Kind::UnstarredOptional) {
            continue;
        }
        let before = at;
        at = optional_start(source, at, !parameter.obey_spaces || control_spaces);
        let bracket = match &parameter.kind {
            Kind::Optional | Kind::UnstarredOptional | Kind::RequiredBracket => {
                if source.as_bytes().get(at) != Some(&b'[') {
                    if matches!(parameter.kind, Kind::RequiredBracket) {
                        return;
                    }
                    at = before;
                    continue;
                }
                true
            }
            Kind::Star => {
                if source.as_bytes().get(at) == Some(&b'*') {
                    at += 1;
                    starred = true;
                    control_spaces = false;
                } else {
                    at = before;
                }
                continue;
            }
            Kind::Flag(flag) => {
                if token(source, at) == Some(flag.as_str()) {
                    if syntax::control_word(flag, 0).is_some() {
                        result.insert(at);
                    }
                    at += flag.len();
                    control_spaces = syntax::control_word(flag, 0).is_some();
                } else {
                    at = before;
                }
                continue;
            }
            Kind::Delimited {
                open,
                close,
                optional,
            } => {
                if token(source, at) != Some(open.as_str()) {
                    if *optional {
                        at = before;
                        continue;
                    }
                    return;
                }
                let Some(end) = operand_delimited_end(source, at, open, close) else {
                    return;
                };
                if syntax::control_word(open, 0).is_some() {
                    result.insert(at);
                }
                if syntax::control_word(close, 0).is_some() {
                    result.insert(end - close.len());
                }
                at = end;
                control_spaces = syntax::control_word(close, 0).is_some();
                continue;
            }
            _ => false,
        };
        if bracket || source.as_bytes().get(at) == Some(&b'{') {
            let group = if bracket && !signature.nested_brackets {
                syntax::bracket_argument(source, at)
            } else {
                syntax::group(
                    source,
                    at,
                    if bracket { b'[' } else { b'{' },
                    if bracket { b']' } else { b'}' },
                )
            };
            let Some((_, end)) = group else { return };
            at = end;
            control_spaces = false;
        } else {
            if matches!(parameter.kind, Kind::RequiredGroup) {
                return;
            }
            let Some(value) = required_token(source, at) else {
                return;
            };
            let word = syntax::control_word(source, at).is_some();
            if word {
                result.insert(at);
            }
            at += value.len();
            control_spaces = word || value == "\\ ";
        }
    }
}

/// Preserve an existing bare argument on this logical line. A normal newline
/// usually means the user is starting a new call, followed by unrelated source
/// such as end{document}; a commented-out newline continues the current call.
fn completion_required_token(source: &str) -> Option<&str> {
    let mut i = 0;
    loop {
        match source.as_bytes().get(i) {
            Some(b' ' | b'\t') => i += 1,
            Some(b'%') => i += source[i..].find('\n')? + 1,
            _ => {
                let value = required_token(source, i)?;
                // These tokens normally close a surrounding math, alignment,
                // group, or environment. Keep placeholders before them when
                // completing an unfinished call.
                return (!matches!(
                    value,
                    "$" | "&"
                        | "\\\\"
                        | "\\)"
                        | "\\]"
                        | "\\right"
                        | "\\end"
                        | "\\endgroup"
                        | "\\egroup"
                        | "\\fi"
                        | "\\else"
                        | "\\or"
                ))
                .then_some(value);
            }
        }
    }
}

fn delimiter_spec(source: &str, start: usize) -> Option<(String, usize)> {
    let start = syntax::skip_space(source, start);
    let (value, end) = if source.as_bytes().get(start) == Some(&b'{') {
        let (group, end) = syntax::group(source, start, b'{', b'}')?;
        (group.value, end)
    } else {
        let value = token(source, start)?;
        (value.to_owned(), start + value.len())
    };
    let at = syntax::skip_space(&value, 0);
    let delimiter = token(&value, at)?;
    if syntax::skip_space(&value, at + delimiter.len()) != value.len()
        || delimiter.chars().any(char::is_whitespace)
        || matches!(delimiter, "{" | "}" | "%" | "#")
    {
        return None;
    }
    Some((delimiter.to_owned(), end))
}

fn signature(workspace: &Workspace, name: &str, scope: &BTreeSet<Url>) -> Option<Signature> {
    lookup(workspace, name, false, scope)
}

fn lookup(
    workspace: &Workspace,
    name: &str,
    environment: bool,
    scope: &BTreeSet<Url>,
) -> Option<Signature> {
    let mut definitions = workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .flat_map(|(uri, doc)| {
            (if environment {
                &doc.syntax.environments
            } else {
                &doc.syntax.definitions
            })
            .iter()
            .filter(move |entry| entry.name == name)
            .map(move |entry| (uri, doc, entry))
        });
    if let Some((uri, doc, entry)) = definitions.next() {
        if definitions.next().is_some() {
            return None;
        }
        definition(uri, doc, entry, environment)
    } else if environment {
        builtin_environment(name)
    } else {
        let mut environments = workspace
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .flat_map(|(uri, doc)| {
                doc.syntax
                    .environments
                    .iter()
                    .filter(move |entry| entry.name == name)
                    .map(move |entry| (uri, doc, entry))
            });
        if let Some((uri, doc, entry)) = environments.next() {
            if environments.next().is_some() {
                return None;
            }
            let mut signature = definition(uri, doc, entry, true)?;
            signature.name = name.into();
            signature.parameters.remove(0);
            Some(signature)
        } else {
            builtin(name)
        }
    }
}

fn builtin_environment(name: &str) -> Option<Signature> {
    let arguments: &[(&str, bool)] = match name {
        "tabular" | "array" => &[("position", true), ("columns", false)],
        "tabular*" => &[("width", false), ("position", true), ("columns", false)],
        "minipage" => &[
            ("position", true),
            ("height", true),
            ("inner position", true),
            ("width", false),
        ],
        "figure" | "figure*" | "table" | "table*" => &[("placement", true)],
        "thebibliography" => &[("widest label", false)],
        "list" => &[("label", false), ("spacing", false)],
        "alignat" | "alignat*" => &[("column pairs", false)],
        "alignedat" => &[("position", true), ("column pairs", false)],
        _ => return None,
    };
    let mut parameters = vec![Parameter {
        obey_spaces: false,
        kind: Kind::Required,
        label: format!("{{{name}}}"),
    }];
    parameters.extend(arguments.iter().map(|(label, optional)| Parameter {
        obey_spaces: false,
        kind: if *optional {
            Kind::Optional
        } else {
            Kind::Required
        },
        label: if *optional {
            format!("[{label}]")
        } else {
            format!("{{{label}}}")
        },
    }));
    Some(Signature {
        name: "begin".into(),
        parameters,
        documentation: format!("Arguments of the {name} environment."),
        starred: false,
        nested_brackets: true,
    })
}

fn group_end(doc: &Document, start: usize, cursor: usize, open: u8, close: u8) -> Option<usize> {
    let source = &doc.text.source;
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut braces = 0usize;
    let mut i = start + 1;
    // Parser-emitted ignored spans are ordered and do not overlap.
    let mut ignored = doc.syntax.ignored.partition_point(|span| span.end <= i);
    while i < cursor {
        while doc
            .syntax
            .ignored
            .get(ignored)
            .is_some_and(|span| span.end <= i)
        {
            ignored += 1;
        }
        if let Some(span) = doc.syntax.ignored.get(ignored) {
            if span.start <= i {
                i = span.end;
                continue;
            }
        }
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            b'{' if open != b'{' => braces += 1,
            b'}' if open != b'{' => braces = braces.saturating_sub(1),
            ch if ch == open && braces == 0 => depth += 1,
            ch if ch == close && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn delimited_end(
    doc: &Document,
    start: usize,
    cursor: usize,
    open: &str,
    close: &str,
) -> Option<usize> {
    let source = &doc.text.source;
    let mut i = start + open.len();
    let mut depth = 1usize;
    let mut braces = 0usize;
    let mut ignored = doc.syntax.ignored.partition_point(|span| span.end <= i);
    while i < cursor {
        while doc
            .syntax
            .ignored
            .get(ignored)
            .is_some_and(|span| span.end <= i)
        {
            ignored += 1;
        }
        if let Some(span) = doc.syntax.ignored.get(ignored) {
            if span.start <= i {
                i = span.end;
                continue;
            }
        }
        let current = token(source, i)?;
        i += current.len();
        if i > cursor {
            return None;
        }
        match current {
            "{" => braces += 1,
            "}" => braces = braces.saturating_sub(1),
            current if current == close && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            current if current == open && braces == 0 => depth += 1,
            _ => {}
        }
    }
    None
}

/// Readers may ignore spaces, including lexical blanks after a control word.
/// A comment removes its endline and the next line's indentation. A genuinely
/// blank line contributes \par even when ordinary space tokens are ignored.
fn optional_start(source: &str, mut i: usize, ignore_spaces: bool) -> usize {
    let bytes = source.as_bytes();
    let mut skip_blanks = ignore_spaces;
    let mut new_line = false;
    loop {
        match bytes.get(i) {
            Some(b'%') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                skip_blanks = true;
                new_line = true;
            }
            Some(b' ' | b'\t') if skip_blanks => i += 1,
            Some(b'\r' | b'\n') if skip_blanks && !new_line => {
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                i += 1;
                new_line = true;
            }
            _ => return i,
        }
    }
}

fn active_parameter(
    doc: &Document,
    command: &syntax::Command,
    end: usize,
    cursor: usize,
    signature: &Signature,
) -> Option<usize> {
    let source = &doc.text.source;
    let mut i = end;
    // Literal control words discard following lexical blanks; named calls
    // through a grouped csuse argument do not. endcsname is a control word too.
    let mut control_word_spaces = end == command.span.end;
    let mut starred = false;
    if signature.starred {
        let at = optional_start(source, i, true);
        if source.as_bytes().get(at) == Some(&b'*') {
            i = at + 1;
            starred = true;
            control_word_spaces = false;
        }
    }
    for (index, parameter) in signature.parameters.iter().enumerate() {
        if starred && matches!(parameter.kind, Kind::UnstarredOptional) {
            continue;
        }
        let before_space = i;
        i = if parameter.obey_spaces {
            optional_start(source, i, control_word_spaces)
        } else {
            optional_start(source, i, true)
        };
        let missing_optional = match &parameter.kind {
            Kind::Optional | Kind::UnstarredOptional => source.as_bytes().get(i) != Some(&b'['),
            Kind::Star => source.as_bytes().get(i) != Some(&b'*'),
            Kind::Flag(flag) => token(source, i) != Some(flag.as_str()),
            Kind::Delimited {
                open,
                optional: true,
                ..
            } => token(source, i) != Some(open.as_str()),
            _ => false,
        };
        if missing_optional && i <= cursor && i < source.len() {
            // Optional lookahead preserves skipped space tokens on a miss.
            // A later ! parameter must see them even if this reader ignored them.
            i = before_space;
            continue;
        }
        if i >= cursor {
            return Some(index);
        }
        let (open, close) = match &parameter.kind {
            Kind::Flag(flag) => {
                if token(source, i) == Some(flag.as_str()) {
                    let end = i + flag.len();
                    if cursor < end {
                        return Some(index);
                    }
                    i = end;
                    control_word_spaces = syntax::control_word(flag, 0).is_some();
                }
                continue;
            }
            Kind::Delimited {
                open,
                close,
                optional,
            } => {
                if token(source, i) != Some(open.as_str()) {
                    if *optional {
                        continue;
                    }
                    return None;
                }
                match delimited_end(doc, i, cursor, open, close) {
                    Some(end) => {
                        i = end;
                        control_word_spaces = syntax::control_word(close, 0).is_some();
                    }
                    None => return Some(index),
                }
                continue;
            }
            Kind::CommandToken if source.as_bytes().get(i) != Some(&b'{') => {
                let end = crate::named_commands::control_argument_end(doc, command, i)?;
                if cursor <= end {
                    return Some(index);
                }
                i = end;
                control_word_spaces = false;
                continue;
            }
            Kind::Star => {
                if source.as_bytes().get(i) == Some(&b'*') {
                    i += 1;
                    starred = true;
                    control_word_spaces = false;
                }
                continue;
            }
            Kind::Optional | Kind::UnstarredOptional if source.as_bytes().get(i) != Some(&b'[') => {
                continue
            }
            Kind::RequiredBracket if source.as_bytes().get(i) != Some(&b'[') => return None,
            Kind::Optional | Kind::UnstarredOptional | Kind::RequiredBracket => (b'[', b']'),
            Kind::Required if source.as_bytes().get(i) != Some(&b'{') => {
                if &source[i..] == "\\" {
                    return Some(index);
                }
                let argument = required_token(source, i)?;
                let end = i + argument.len();
                let word = syntax::control_word(argument, 0).is_some();
                // At EOF the user may still be typing this control word. A
                // lexical separator confirms that the argument has finished.
                if cursor < end || (word && cursor == end && end == source.len()) {
                    return Some(index);
                }
                i = end;
                // Both control words and the control-space token discard
                // following lexical blanks; other control symbols do not.
                control_word_spaces = word || argument == "\\ ";
                continue;
            }
            Kind::RequiredGroup if source.as_bytes().get(i) != Some(&b'{') => return None,
            Kind::Required | Kind::RequiredGroup | Kind::CommandToken => (b'{', b'}'),
        };
        let end = if open == b'[' && !signature.nested_brackets {
            syntax::optional_note_end(&source[..cursor], i)
        } else {
            group_end(doc, i, cursor, open, close)
        };
        match end {
            Some(end) => {
                i = end;
                control_word_spaces = false;
            }
            None => return Some(index),
        }
    }
    None
}

pub fn help(workspace: &Workspace, uri: &Url, position: Position) -> Option<SignatureHelp> {
    let doc = workspace.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex {
        return None;
    }
    let cursor = doc.text.offset(position)?;
    if doc.syntax.ignored.iter().any(|span| {
        span.start <= cursor.saturating_sub(1)
            && cursor.saturating_sub(1) < span.end
            && !doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "url" | "href" | "hyperref")
                    && command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.span == *span)
            })
    }) {
        return None;
    }
    let scope = workspace.symbol_scope(uri);
    for (index, command) in doc
        .syntax
        .commands
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, command)| command.span.end <= cursor)
        .take(128)
    {
        let literal = if command.name == "endcsname" {
            index.checked_sub(1).and_then(|index| {
                syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index)
            })
        } else {
            None
        };
        if doc.syntax.definitions.iter().any(|entry| {
            entry.span == command.span || literal.as_ref().is_some_and(|span| entry.span == *span)
        }) {
            continue;
        }
        let named = command.argument.as_ref().filter(|argument| {
            command.name == "csuse"
                && cursor >= argument.outer_span.end
                && syntax::is_static_key(&argument.value)
        });
        let end = named.map_or(command.span.end, |argument| argument.outer_span.end);
        let name = named
            .map(|argument| argument.value.as_str())
            .or_else(|| literal.as_ref().map(|span| &doc.text.source[span.clone()]))
            .unwrap_or(command.name.as_str());
        let signature = if name == "hyperref"
            && doc
                .text
                .source
                .as_bytes()
                .get(syntax::skip_space(&doc.text.source, end))
                == Some(&b'{')
            && !workspace.documents.iter().any(|(uri, doc)| {
                scope.contains(uri)
                    && doc
                        .syntax
                        .definitions
                        .iter()
                        .any(|entry| entry.name == name)
            }) {
            Some(Signature {
                name: name.into(),
                starred: false,
                nested_brackets: true,
                documentation: "Link to a URL using a category and target name. Requires hyperref."
                    .into(),
                parameters: ["url", "category", "name", "text"]
                    .into_iter()
                    .map(|label| Parameter {
                        obey_spaces: false,
                        kind: Kind::Required,
                        label: format!("{{{label}}}"),
                    })
                    .collect(),
            })
        } else if name == "begin" {
            syntax::group(
                &doc.text.source,
                syntax::skip_space(&doc.text.source, end),
                b'{',
                b'}',
            )
            .map(|(argument, _)| argument)
            .filter(|argument| cursor > argument.span.end)
            .and_then(|argument| lookup(workspace, &argument.value, true, &scope))
            .or_else(|| signature(workspace, name, &scope))
        } else {
            signature(workspace, name, &scope)
        };
        if let Some(mut signature) = signature {
            if literal.is_some() {
                signature.name = format!("csname {name}\\endcsname");
            } else if named.is_some() {
                signature.name = format!("csuse{{{name}}}");
            }
            if let Some(active) = active_parameter(doc, command, end, cursor, &signature) {
                return Some(signature.information(active));
            }
        }
    }
    None
}
