//! Linearly match grouped call arguments; do not execute macro definitions.
use crate::{
    catalog::COMMANDS,
    syntax::{self, Entry},
    workspace::{Document, DocumentKind, Workspace},
};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::*;

#[derive(Clone)]
enum Kind {
    Required,
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
                            documentation: None,
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
    // The same argument reader decides whether existing custom delimiters must
    // be preserved. Stop at the first required argument: later delimiters cannot
    // start a valid call here.
    for parameter in &signature.parameters {
        match &parameter.kind {
            Kind::Delimited { open, optional, .. } => {
                if token(following, 0) == Some(open.as_str()) {
                    return Some((signature.label(), None));
                }
                if !optional {
                    break;
                }
            }
            Kind::Optional | Kind::UnstarredOptional | Kind::Star => {}
            Kind::Flag(flag) => {
                if token(following, 0) == Some(flag.as_str()) {
                    return Some((signature.label(), None));
                }
            }
            _ => break,
        }
    }
    let mut snippet = name.to_owned();
    let mut tabstop = 0;
    for (index, parameter) in signature.parameters.iter().enumerate() {
        let (open, close) = match &parameter.kind {
            Kind::Required | Kind::CommandToken => ("{", "}"),
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
        "includegraphics" => (&[("options", true), ("file", false)], true),
        "documentclass" | "usepackage" | "RequirePackage" | "LoadClass" => (
            &[
                ("options", true),
                ("name", false),
                ("minimum version", true),
            ],
            false,
        ),
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
        name if syntax::is_reference(name) => {
            (&[("keys", false)], syntax::reference_accepts_star(name))
        }
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
        nested_brackets: !(syntax::is_citation(name)
            || syntax::reference_note_count(name) > 0
            || matches!(
                name,
                "hyperref"
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
    let source = &doc.text.source;
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let mut i = syntax::skip_space(source, entry.span.end);
    if !environment
        && !crate::named_commands::declares(declaration)
        && source.as_bytes().get(entry.span.start) != Some(&b'\\')
    {
        let end = doc
            .syntax
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
                kind: Kind::Star,
                label: "*".into(),
            });
            parameters.push(Parameter {
                kind: Kind::UnstarredOptional,
                label: "[size]".into(),
            });
            for number in 1..=count {
                parameters.push(Parameter {
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
            while i < source.len() {
                i = syntax::skip_space(source, i);
                let Some(&kind) = source.as_bytes().get(i) else {
                    break;
                };
                i += 1;
                if matches!(kind, b'+' | b'!') {
                    continue;
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
                parameters.push(Parameter { kind, label });
                if parameters.len() > 9 {
                    return None;
                }
            }
        }
        "newtheorem" => {
            nested_brackets = false;
            parameters.push(Parameter {
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
    if parameters.is_empty() {
        return None;
    }
    if environment {
        parameters.insert(
            0,
            Parameter {
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
        documentation: format!(
            "{}\n{}:{}",
            entry.detail,
            uri.path(),
            doc.text.position(entry.span.start).line + 1
        ),
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
        kind: Kind::Required,
        label: format!("{{{name}}}"),
    }];
    parameters.extend(arguments.iter().map(|(label, optional)| Parameter {
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

fn active_parameter(
    doc: &Document,
    command: &syntax::Command,
    end: usize,
    cursor: usize,
    signature: &Signature,
) -> Option<usize> {
    let source = &doc.text.source;
    let mut i = end;
    let mut starred = false;
    if signature.starred {
        i = syntax::skip_space(source, i);
    }
    if signature.starred && source.as_bytes().get(i) == Some(&b'*') {
        i += 1;
        starred = true;
    }
    for (index, parameter) in signature.parameters.iter().enumerate() {
        i = syntax::skip_space(source, i);
        if starred && matches!(parameter.kind, Kind::UnstarredOptional) {
            continue;
        }
        if i == cursor {
            match &parameter.kind {
                Kind::Optional | Kind::UnstarredOptional
                    if source.as_bytes().get(i).is_some_and(|byte| *byte != b'[') =>
                {
                    continue
                }
                Kind::Star if source.as_bytes().get(i).is_some_and(|byte| *byte != b'*') => {
                    continue
                }
                Kind::Delimited {
                    open,
                    optional: true,
                    ..
                } if token(source, i).is_some_and(|value| value != open) => continue,
                Kind::Flag(flag) if token(source, i).is_some_and(|value| value != flag) => continue,
                _ => {}
            }
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
                    Some(end) => i = end,
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
                continue;
            }
            Kind::Star => {
                if source.as_bytes().get(i) == Some(&b'*') {
                    i += 1;
                    starred = true;
                }
                continue;
            }
            Kind::Optional | Kind::UnstarredOptional if source.as_bytes().get(i) != Some(&b'[') => {
                continue
            }
            Kind::RequiredBracket if source.as_bytes().get(i) != Some(&b'[') => return None,
            Kind::Optional | Kind::UnstarredOptional | Kind::RequiredBracket => (b'[', b']'),
            Kind::Required if source.as_bytes().get(i) != Some(&b'{') => return None,
            Kind::Required | Kind::CommandToken => (b'{', b'}'),
        };
        let end = if open == b'[' && !signature.nested_brackets {
            syntax::optional_note_end(&source[..cursor], i)
        } else {
            group_end(doc, i, cursor, open, close)
        };
        match end {
            Some(end) => i = end,
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
                && syntax::is_static_key(argument.value.trim())
        });
        let end = named.map_or(command.span.end, |argument| argument.outer_span.end);
        let name = named
            .map(|argument| argument.value.trim())
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
