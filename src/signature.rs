//! Linearly match call arguments; do not execute macro definitions.
use crate::{
    catalog::COMMANDS,
    syntax::{self, required_token, token, Entry},
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    borrow::Cow,
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};
use tower_lsp::lsp_types::*;

#[derive(Clone, PartialEq)]
enum Kind {
    Required,
    RequiredGroup,
    CitationKeys(crate::citations::Style),
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

#[derive(Clone, PartialEq)]
struct Parameter {
    obey_spaces: bool,
    kind: Kind,
    label: String,
}

#[derive(Clone, PartialEq)]
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
    CompletionSignatures::new(workspace, scope).at(name, "")
}

/// Reuse lazy alias resolution across all candidates of one completion request.
pub(crate) struct CompletionSignatures<'a> {
    workspace: &'a Workspace,
    scope: &'a BTreeSet<Url>,
    aliases: OnceCell<CommandSignatures<'a>>,
}

impl<'a> CompletionSignatures<'a> {
    pub(crate) fn new(workspace: &'a Workspace, scope: &'a BTreeSet<Url>) -> Self {
        Self {
            workspace,
            scope,
            aliases: OnceCell::new(),
        }
    }

    pub(crate) fn at(&self, name: &str, following: &str) -> Option<(String, Option<String>)> {
        let signature = lookup(self.workspace, name, false, self.scope, Some(&self.aliases))?;
        if existing_arguments(&signature, following) {
            return Some((signature.label(), None));
        }
        let mut snippet = name.to_owned();
        let mut tabstop = 0;
        for (index, parameter) in signature.parameters.iter().enumerate() {
            let (open, close) = match &parameter.kind {
                Kind::Required
                | Kind::RequiredGroup
                | Kind::CitationKeys(_)
                | Kind::CommandToken => ("{", "}"),
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
    if let Some(style) = crate::citations::style(name) {
        return Some(Signature {
            name: name.into(),
            parameters: citation_parameters(style, 1),
            documentation: if style.category_membership() {
                "Assign bibliography keys to a declared category. This does not cite the entries. Requires biblatex."
            } else if style.set_definition() {
                "Define a dynamic bibliography set with a citation key and a list of member keys. Requires biblatex with Biber."
            } else if style.context_codes() == 2 {
                "Test a bibliography entry by key and provide code for its two branches. Requires biblatex."
            } else if style.single_key() {
                "Execute code in one bibliography entry's data context. Supports the starred form. Requires biblatex."
            } else {
                "Biblatex citation arguments. Repeated calls accept up to two overall notes in parentheses; volume citations separate the volume and optional pages from bibliography keys. Data citations select a field or list after an optional format. Requires biblatex."
            }.into(),
            starred: style.accepts_star(),
            nested_brackets: false,
        });
    }
    let (arguments, starred): (&[(&str, bool)], bool) = match name {
        "DeclareBibliographyCategory" => (&[("category", false)], false),
        "printfield" => (&[("format", true), ("field", false)], false),
        "thefield" | "strfield" => (&[("field", false)], false),
        "iffieldundef" => (
            &[("field", false), ("true", false), ("false", false)],
            false,
        ),
        "iflistundef" => (
            &[("literal list", false), ("true", false), ("false", false)],
            false,
        ),
        "ifnameundef" => (
            &[("name list", false), ("true", false), ("false", false)],
            false,
        ),
        "ifentrytype" => (
            &[("entry type", false), ("true", false), ("false", false)],
            false,
        ),
        "iffieldequalstr" => (
            &[
                ("field", false),
                ("string", false),
                ("true", false),
                ("false", false),
            ],
            false,
        ),
        "iffieldequals" => (
            &[
                ("field", false),
                ("command", false),
                ("true", false),
                ("false", false),
            ],
            false,
        ),
        "iffieldsequal" => (
            &[
                ("field 1", false),
                ("field 2", false),
                ("true", false),
                ("false", false),
            ],
            false,
        ),
        "iflistequals" => (
            &[
                ("literal list", false),
                ("command", false),
                ("true", false),
                ("false", false),
            ],
            false,
        ),
        "ifnameequals" => (
            &[
                ("name list", false),
                ("command", false),
                ("true", false),
                ("false", false),
            ],
            false,
        ),
        "printlist" => (
            &[
                ("format", true),
                ("start-stop", true),
                ("literal list", false),
            ],
            false,
        ),
        "printnames" => (
            &[("format", true), ("start-stop", true), ("name list", false)],
            false,
        ),
        name if syntax::declares_traditional_command(name) => (
            &[
                ("command", false),
                ("arguments", true),
                ("default", true),
                ("definition", false),
            ],
            true,
        ),
        "newenvironment" | "renewenvironment" | "provideenvironment" => (
            &[
                ("environment", false),
                ("arguments", true),
                ("default", true),
                ("begin", false),
                ("end", false),
            ],
            true,
        ),
        "NewDocumentEnvironment"
        | "RenewDocumentEnvironment"
        | "ProvideDocumentEnvironment"
        | "DeclareDocumentEnvironment" => (
            &[
                ("environment", false),
                ("arguments", false),
                ("begin", false),
                ("end", false),
            ],
            false,
        ),
        "NewEnviron" | "RenewEnviron" => (
            &[
                ("environment", false),
                ("arguments", true),
                ("default", true),
                ("definition", false),
                ("final code", true),
            ],
            false,
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
        "mintinline" | "mint" => (
            &[("options", true), ("language", false), ("code", false)],
            false,
        ),
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
        "undef" | "gundef" | "csuse" | "csmeaning" | "csundef" | "csgundef" => {
            (&[("command", false)], false)
        }
        "cslet" | "letcs" | "csletcs" => (&[("alias", false), ("command", false)], false),
        "csappto" | "cseappto" | "csgappto" | "csxappto" | "cspreto" | "csepreto" | "csgpreto"
        | "csxpreto" => (&[("command", false), ("code", false)], false),
        "ifcsdef" | "ifcsundef" | "ifcsvoid" | "ifcsmacro" | "ifcsparam" | "ifcsprotected"
        | "ifcsltxprotect" | "ifcscounter" | "ifcslength" | "ifcsdimen" | "ifdefcounter"
        | "ifdeflength" | "ifdefdimen" => (
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
        "addbibresource" | "addglobalbib" => (&[("options", true), ("file", false)], false),
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
        "ifstrequal" => (
            &[
                ("string", false),
                ("string", false),
                ("then", false),
                ("else", false),
            ],
            false,
        ),
        "ifstrempty" | "ifblank" | "notblank" => (
            &[("string", false), ("then", false), ("else", false)],
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
        "deflength" => (&[("length", false), ("expression", false)], false),
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
        "DeclareFontEncoding" => (
            &[
                ("encoding", false),
                ("text settings", false),
                ("math settings", false),
            ],
            false,
        ),
        "DeclareFontEncodingDefaults" => {
            (&[("text settings", false), ("math settings", false)], false)
        }
        "DeclareFontFamily" => (
            &[
                ("encoding", false),
                ("family", false),
                ("loading settings", false),
            ],
            false,
        ),
        "DeclareFontShape" => (
            &[
                ("encoding", false),
                ("family", false),
                ("series", false),
                ("shape", false),
                ("size specification", false),
                ("loading settings", false),
            ],
            false,
        ),
        "DeclareFontSubstitution" => (
            &[
                ("encoding", false),
                ("family", false),
                ("series", false),
                ("shape", false),
            ],
            false,
        ),
        "DeclareMathSizes" => (
            &[
                ("text size", false),
                ("math size", false),
                ("script size", false),
                ("scriptscript size", false),
            ],
            true,
        ),
        "fontencoding" => (&[("encoding", false)], false),
        "fontfamily" => (&[("family", false)], false),
        "fontseries" => (&[("series", false)], false),
        "fontshape" => (&[("shape", false)], false),
        "fontsize" => (&[("size", false), ("baseline skip", false)], false),
        "linespread" => (&[("factor", false)], false),
        "usefont" => (
            &[
                ("encoding", false),
                ("family", false),
                ("series", false),
                ("shape", false),
            ],
            false,
        ),
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
        "defcounter" => (&[("counter", false), ("expression", false)], false),
        "ifltxcounter" => (
            &[("counter", false), ("then", false), ("else", false)],
            false,
        ),
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
                || (matches!(
                    name,
                    "letcs"
                        | "undef"
                        | "gundef"
                        | "deflength"
                        | "ifdefcounter"
                        | "ifdeflength"
                        | "ifdefdimen"
                ) || syntax::declares_traditional_command(name)
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
                || crate::minted::is_inline(name)
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
            || crate::minted::is_inline(name)
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
                    | "printfield"
                    | "printlist"
                    | "printnames"
                    | "newenvironment"
                    | "renewenvironment"
                    | "provideenvironment"
                    | "NewEnviron"
                    | "RenewEnviron"
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
    let signature = source_definition(&doc.text.source, &doc.syntax, entry, environment)?;
    describe_definition(uri, doc, entry, environment, signature)
}

fn describe_definition(
    uri: &Url,
    doc: &Document,
    entry: &Entry,
    environment: bool,
    mut signature: Signature,
) -> Option<Signature> {
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

fn definition_arguments_start(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
    environment: bool,
) -> Option<usize> {
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
    Some(i)
}

/// Comments remove their endline and indentation, but ordinary spaces between
/// digits remain significant. Do not expand macros or discard escaped tokens.
fn literal_argument_count(source: &str) -> Option<usize> {
    if !source.contains('%') {
        return source.trim().parse().ok();
    }
    let mut literal = String::new();
    let mut at = 0;
    while at < source.len() {
        at = tex_operand_comments(source, at);
        let Some(ch) = source[at..].chars().next() else {
            break;
        };
        if ch == '\\' {
            return None;
        }
        literal.push(ch);
        at += ch.len_utf8();
    }
    literal.trim().parse().ok()
}

fn source_definition(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
    environment: bool,
) -> Option<Signature> {
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let mut i = definition_arguments_start(source, parsed, entry, environment)?;
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
                let count = literal_argument_count(&argument.value)?;
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
            let count = literal_argument_count(&count.value)?;
            if count > 9 {
                return None;
            }
            i = syntax::skip_space(source, end);
            let default = syntax::bracket_argument(source, i).map(|(argument, _)| argument.value);
            if count == 0 && default.is_some() {
                return None;
            }
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

/// The start control of an environment reads its arguments after the name
/// supplied to begin. Keep signature help's synthetic name parameter separate.
fn environment_start_signature(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
) -> Option<Signature> {
    let mut signature = source_definition(source, parsed, entry, true)?;
    // A traditional declaration without a count returns an empty signature
    // before the synthetic name parameter is inserted.
    if signature.name == "begin" && !signature.parameters.is_empty() {
        signature.parameters.remove(0);
    }
    signature.name = entry.name.clone();
    Some(signature)
}

fn body_alias(source: &str, body: &syntax::Argument) -> Option<String> {
    let start = syntax::skip_space(source, body.span.start);
    let word = syntax::control_word(source, start)?;
    (syntax::skip_space(source, word.end) == body.span.end)
        .then(|| source[word.start + 1..word.end].into())
}

fn environment_bodies(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
) -> Option<(syntax::Argument, syntax::Argument)> {
    let mut at = definition_arguments_start(source, parsed, entry, true)?;
    match entry.detail.strip_prefix("Defined with \\")? {
        "newenvironment" | "renewenvironment" | "provideenvironment" => {
            if let Some((_, end)) = syntax::bracket_argument(source, at) {
                at = syntax::skip_space(source, end);
                if let Some((_, end)) = syntax::bracket_argument(source, at) {
                    at = syntax::skip_space(source, end);
                }
            }
        }
        "NewDocumentEnvironment"
        | "RenewDocumentEnvironment"
        | "ProvideDocumentEnvironment"
        | "DeclareDocumentEnvironment" => {
            at = syntax::group(source, at, b'{', b'}')?.1;
        }
        _ => return None,
    }
    let (begin, end) = syntax::group(source, syntax::skip_space(source, at), b'{', b'}')?;
    let (end, _) = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?;
    Some((begin, end))
}

/// A body containing exactly one control word forwards to its current meaning.
/// This recognizes lexical aliases without evaluating arbitrary replacement text.
fn source_alias(source: &str, parsed: &syntax::Syntax, entry: &Entry) -> Option<String> {
    body_alias(source, &source_alias_body(source, parsed, entry)?)
}

/// Read a static copy's source and operand end. A copy captures a meaning;
/// possible source shapes do not certify that meaning or a late-bound alias.
pub(crate) fn literal_copy_target(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
    environment: bool,
) -> Option<(String, usize)> {
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let named = if environment {
        syntax::declares_environment_copy(declaration).then_some(true)?
    } else {
        match declaration {
            "let" | "cslet" => false,
            "letcs" | "csletcs" => true,
            name if syntax::declares_command_copy(name) => false,
            _ => return None,
        }
    };
    let mut at = if declaration == "let" {
        // Primitive let does not accept a braced destination or source. Do
        // not mistake a copied closing brace for the destination's delimiter.
        let mut at = syntax::skip_space(source, entry.span.end);
        if source.as_bytes().get(entry.span.start) != Some(&b'\\') {
            at = parsed
                .commands
                .iter()
                .find(|command| command.span.start == at && command.name == "endcsname")?
                .span
                .end;
        }
        at = syntax::skip_space(source, at);
        if source.as_bytes().get(at) == Some(&b'=') {
            at = syntax::skip_space(source, at + 1);
        }
        at
    } else {
        definition_arguments_start(source, parsed, entry, environment)?
    };
    if named {
        let (argument, end) = syntax::group(source, at, b'{', b'}')?;
        return syntax::is_static_key(&argument.value).then_some((argument.value, end));
    }
    if declaration != "let" {
        if let Some((argument, end)) = syntax::group(source, at, b'{', b'}') {
            at = syntax::skip_space(source, argument.span.start);
            let word = syntax::control_word(source, at)?;
            if syntax::skip_space(source, word.end) != argument.span.end {
                return None;
            }
            return Some((source[word.start + 1..word.end].into(), end));
        }
    }
    let word = syntax::control_word(source, at)?;
    Some((source[word.start + 1..word.end].into(), word.end))
}

/// Captured parameter grammar for a command's current, unexpanded meaning.
#[derive(Clone, PartialEq)]
pub(crate) struct CommandShape {
    signature: Option<Signature>,
    tex: Option<TexOperands>,
}

impl std::fmt::Debug for CommandShape {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandShape")
            .field("signature", &self.signature.as_ref().map(Signature::label))
            .field("tex", &self.tex.is_some())
            .finish()
    }
}

impl CommandShape {
    /// An undefined control consumes no operands; retaining its former alias
    /// grammar would hide the following executable token or file load.
    pub(crate) fn no_arguments() -> Self {
        Self {
            signature: None,
            tex: Some(TexOperands {
                prefix: vec![],
                delimiters: vec![],
            }),
        }
    }

    fn takes_no_arguments(&self) -> bool {
        self.tex
            .as_ref()
            .is_some_and(|tex| tex.prefix.is_empty() && tex.delimiters.is_empty())
            || self
                .signature
                .as_ref()
                .is_some_and(|signature| signature.parameters.is_empty())
    }

    pub(crate) fn definition(source: &str, parsed: &syntax::Syntax, entry: &Entry) -> Option<Self> {
        let mut signature = source_definition(source, parsed, entry, false);
        let definition = TexOperands::definition(source, parsed, entry);
        let known = definition.is_some();
        let tex = definition
            .filter(|(operands, _)| operands.delimited() || signature.is_none())
            .map(|(operands, _)| operands);
        if TexOperands::declares(entry) && (tex.is_some() || !known) {
            signature = None;
        }
        (signature.is_some() || tex.is_some()).then_some(Self { signature, tex })
    }

    fn argument_tokens(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        command: &syntax::Command,
        literal: Option<usize>,
        result: &mut BTreeSet<usize>,
    ) {
        if let Some(end) = self
            .tex
            .as_ref()
            .and_then(|operands| operands.end(source, command.span.end, true))
        {
            mark_operand_tokens(parsed, command.span.end, end, literal, result);
        } else if let Some(signature) = &self.signature {
            argument_tokens(
                (source, parsed),
                command.span.end,
                true,
                signature,
                false,
                literal,
                result,
            );
        } else {
            unknown_argument_token(source, command.span.end, parsed, literal, result);
        }
    }
}

/// Tokens declaring an unexpanded, argument-free file loader alias.
/// Its body forwards the loader rather than performing a dynamic file load.
pub(crate) fn file_load_alias_body_positions(
    source: &str,
    parsed: &syntax::Syntax,
) -> BTreeSet<usize> {
    parsed
        .definitions
        .iter()
        .filter_map(|entry| {
            if matches!(
                entry.detail.strip_prefix("Defined with \\"),
                Some("edef" | "xdef" | "csedef" | "csxdef")
            ) {
                return None;
            }
            let signature = source_definition(source, parsed, entry, false)?;
            if !signature.parameters.is_empty() {
                return None;
            }
            let body = source_alias_body(source, parsed, entry)?;
            let target = body_alias(source, &body)?;
            (matches!(target.as_str(), "input" | "include")
                || syntax::loader_extension(&target).is_some())
            .then(|| syntax::skip_space(source, body.span.start))
        })
        .collect()
}

fn source_alias_body(
    source: &str,
    parsed: &syntax::Syntax,
    entry: &Entry,
) -> Option<syntax::Argument> {
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let mut at = definition_arguments_start(source, parsed, entry, false)?;
    if syntax::declares_traditional_command(declaration)
        || matches!(
            declaration,
            "DeclareTextCommand"
                | "ProvideTextCommand"
                | "DeclareTextCommandDefault"
                | "ProvideTextCommandDefault"
        )
    {
        if let Some((_, end)) = syntax::bracket_argument(source, at) {
            at = syntax::skip_space(source, end);
        }
    } else if syntax::declares_document_command(declaration) {
        at = syntax::group(source, at, b'{', b'}')?.1;
    } else if !matches!(
        declaration,
        "def" | "gdef" | "edef" | "xdef" | "csdef" | "csgdef" | "csedef" | "csxdef"
    ) {
        return None;
    }
    syntax::group(source, syntax::skip_space(source, at), b'{', b'}').map(|(body, _)| body)
}

/// Definition bodies are not executed where they occur. Keep their names
/// uncertain during environment matching without excluding rename references.
pub(crate) fn definition_body_spans(
    source: &str,
    parsed: &syntax::Syntax,
) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    for entry in &parsed.definitions {
        let body = (|| {
            if TexOperands::declares(entry) {
                let (_, at) = TexOperands::definition(source, parsed, entry)?;
                return syntax::group(source, at, b'{', b'}').map(|(body, _)| body);
            }
            let declaration = entry.detail.strip_prefix("Defined with \\")?;
            let mut at = definition_arguments_start(source, parsed, entry, false)?;
            if syntax::declares_traditional_command(declaration) {
                for _ in 0..2 {
                    if let Some((_, end)) = syntax::bracket_argument(source, at) {
                        at = syntax::skip_space(source, end);
                    }
                }
            } else if syntax::declares_document_command(declaration) {
                at = syntax::group(source, at, b'{', b'}')?.1;
            } else {
                return None;
            }
            syntax::group(source, syntax::skip_space(source, at), b'{', b'}').map(|(body, _)| body)
        })();
        if let Some(body) = body {
            spans.push(body.span);
        }
    }
    for entry in &parsed.environments {
        if let Some((begin, end)) = environment_bodies(source, parsed, entry) {
            spans.extend([begin.span, end.span]);
        }
    }
    spans
}

fn short(value: &str) -> String {
    let mut literal = String::new();
    let mut at = 0;
    let mut control_word = false;
    let mut pending_space = false;
    while at < value.len() {
        if value.as_bytes()[at] == b'%' {
            at = tex_operand_comments(value, at);
            // Removing a comment must not turn two tokens into a longer
            // control word when the preview is read as TeX source.
            if control_word
                && value[at..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_alphabetic() || ch == '@')
            {
                pending_space = true;
            }
            continue;
        }
        let end = if value.as_bytes()[at] == b'\\' {
            let word = syntax::control_word(value, at);
            let end = word
                .as_ref()
                .map(|word| word.end)
                .or_else(|| syntax::token(value, at).map(|token| at + token.len()))
                .unwrap_or(at + 1);
            control_word = word.is_some();
            end
        } else {
            let ch = value[at..].chars().next().unwrap();
            control_word = false;
            if ch.is_whitespace() {
                pending_space = true;
                at += ch.len_utf8();
                continue;
            }
            at + ch.len_utf8()
        };
        if pending_space && !literal.is_empty() && !literal.ends_with(' ') {
            literal.push(' ');
        }
        pending_space = false;
        literal.push_str(&value[at..end]);
        at = end;
    }
    literal.chars().take(80).collect()
}

/// Read one lexical TeX token without expanding it. In particular, a delimiter
/// control word must not match a prefix of a longer control word.
/// Static TeX parameter text, used only to establish which tokens are operands.
/// Signature help deliberately keeps its existing, smaller set of signatures.
#[derive(Clone, PartialEq)]
struct TexOperands {
    prefix: Vec<String>,
    delimiters: Vec<Vec<String>>,
}

impl TexOperands {
    fn declares(entry: &Entry) -> bool {
        matches!(
            entry.detail.strip_prefix("Defined with \\"),
            Some("def" | "gdef" | "edef" | "xdef" | "csdef" | "csgdef" | "csedef" | "csxdef")
        )
    }

    fn delimited(&self) -> bool {
        !self.prefix.is_empty()
            || self
                .delimiters
                .iter()
                .any(|delimiter| !delimiter.is_empty())
    }

    fn definition(source: &str, parsed: &syntax::Syntax, entry: &Entry) -> Option<(Self, usize)> {
        if !Self::declares(entry) {
            return None;
        }
        let mut at = entry.span.end;
        let named = entry.detail.starts_with("Defined with \\cs");
        if named {
            // A closing name group leaves the mouth in mid-line state.
            // Unlike a literal control word, it does not discard a prefix space.
            if source.as_bytes().get(at) != Some(&b'}') {
                return None;
            }
            at += 1;
        } else {
            if source.as_bytes().get(entry.span.start) != Some(&b'\\') {
                let start = syntax::skip_space(source, at);
                at = parsed
                    .commands
                    .iter()
                    .find(|command| command.span.start == start && command.name == "endcsname")?
                    .span
                    .end;
            }
            at = optional_start(source, at, true);
        }
        let mut prefix = vec![];
        let mut delimiters: Vec<Vec<String>> = vec![];
        let mut tokens = 0;
        loop {
            at = tex_operand_comments(source, at);
            match source.as_bytes().get(at)? {
                b'{' => break,
                b'#' => {
                    let number = delimiters.len() + 1;
                    let (digit, end) = tex_operand_token(source, at + 1)?;
                    if number > 9 || digit.as_bytes() != [b'0' + number as u8] {
                        return None;
                    }
                    delimiters.push(vec![]);
                    at = end;
                }
                _ => {
                    let (value, end) = tex_operand_token(source, at)?;
                    tokens += 1;
                    if tokens > 64 {
                        return None;
                    }
                    let target = delimiters.last_mut().unwrap_or(&mut prefix);
                    target.push(value.to_owned());
                    at = end;
                }
            }
        }
        syntax::group(source, at, b'{', b'}')?;
        Some((Self { prefix, delimiters }, at))
    }

    fn end(&self, source: &str, at: usize, control_spaces: bool) -> Option<usize> {
        let mut at = tex_operand_match(
            source,
            optional_start(source, at, control_spaces),
            &self.prefix,
        )?;
        for delimiter in &self.delimiters {
            if delimiter.is_empty() {
                at = tex_operand_skip_spaces(source, at)?;
                at = if source.as_bytes().get(at) == Some(&b'{') {
                    syntax::group(source, at, b'{', b'}')?.1
                } else {
                    tex_operand_token(source, at)?.1
                };
            } else {
                loop {
                    at = tex_operand_comments(source, at);
                    if let Some(end) = tex_operand_match(source, at, delimiter) {
                        at = end;
                        break;
                    }
                    at = match source.as_bytes().get(at)? {
                        b'{' => syntax::group(source, at, b'{', b'}')?.1,
                        _ => tex_operand_token(source, at)?.1,
                    };
                }
            }
        }
        Some(at)
    }
}

fn tex_operand_comments(source: &str, mut at: usize) -> usize {
    while source.as_bytes().get(at) == Some(&b'%') {
        at = source[at..]
            .find('\n')
            .map_or(source.len(), |end| at + end + 1);
        while matches!(source.as_bytes().get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
    }
    at
}

fn tex_operand_blank_line(source: &str, at: usize) -> bool {
    let horizontal = |byte: &u8| matches!(byte, b' ' | b'\t');
    let before = source.as_bytes()[..at]
        .iter()
        .rev()
        .find(|byte| !horizontal(byte));
    let after = source.as_bytes()[at..]
        .iter()
        .find(|byte| !horizontal(byte));
    matches!(before, None | Some(b'\r' | b'\n')) && matches!(after, Some(b'\r' | b'\n'))
}

/// Normalize one ordinary TeX space token, retaining its source end. Comments
/// remove the endline and indentation; lexical blanks after a control word (or
/// control-space) disappear. Paragraph tokens and catcode spellings stay unknown.
fn tex_operand_token(source: &str, at: usize) -> Option<(&str, usize)> {
    let at = tex_operand_comments(source, at);
    if matches!(
        source.as_bytes().get(at),
        Some(b' ' | b'\t' | b'\r' | b'\n')
    ) {
        if tex_operand_blank_line(source, at) {
            return None;
        }
        return Some((" ", optional_start(source, at, true)));
    }
    let value = required_token(source, at)?;
    let end = at + value.len();
    let end = if syntax::control_word(value, 0).is_some() || value == "\\ " {
        optional_start(source, end, true)
    } else {
        end
    };
    Some((value, end))
}

fn tex_operand_skip_spaces(source: &str, mut at: usize) -> Option<usize> {
    loop {
        at = tex_operand_comments(source, at);
        if !matches!(
            source.as_bytes().get(at),
            Some(b' ' | b'\t' | b'\r' | b'\n')
        ) {
            return Some(at);
        }
        at = tex_operand_token(source, at)?.1;
    }
}

fn tex_operand_match(source: &str, mut at: usize, pattern: &[String]) -> Option<usize> {
    for expected in pattern {
        let (value, end) = tex_operand_token(source, at)?;
        if value != expected {
            return None;
        }
        at = end;
    }
    Some(at)
}

/// Resolve command arities once across the documents in the analyzed project.
pub(crate) struct CommandSignatures<'a> {
    local: BTreeMap<Cow<'a, str>, Option<Signature>>,
    tex_operands: BTreeMap<Cow<'a, str>, Option<TexOperands>>,
    literal_operands: BTreeMap<Cow<'a, str>, Vec<LiteralOperands>>,
    detokenize_boundary_known: bool,
}

/// Possible declarations only exclude literal edits; they never establish a
/// unique command meaning for navigation, source loading or signature help.
enum LiteralOperands {
    Signature(Signature),
    Tex(TexOperands),
    Alias(String),
    Copy(String),
    Unknown,
}

fn insert_literal_operands<'a>(
    entries: &mut BTreeMap<Cow<'a, str>, Vec<LiteralOperands>>,
    name: Cow<'a, str>,
    signature: Option<&Signature>,
    tex: Option<&TexOperands>,
    alias: Option<&str>,
) {
    let operands = if let Some(alias) = alias {
        Some(LiteralOperands::Alias(alias.into()))
    } else if let Some(tex) = tex {
        Some(LiteralOperands::Tex(tex.clone()))
    } else {
        signature.cloned().map(LiteralOperands::Signature)
    };
    entries
        .entry(name)
        .or_default()
        .push(operands.unwrap_or(LiteralOperands::Unknown));
}

#[derive(Clone)]
pub(crate) struct EnvironmentCall {
    pub argument: syntax::Argument,
    pub span: std::ops::Range<usize>,
    pub begin: bool,
    pub direct: bool,
    pub operand: bool,
    pub deferred: bool,
}

/// Preserve dynamic name arguments as boundaries for ordered pairing.
pub(crate) struct EnvironmentBoundary {
    pub argument: Option<syntax::Argument>,
    pub span: std::ops::Range<usize>,
    pub begin: bool,
    pub direct: bool,
    pub operand: bool,
    pub deferred: bool,
}

impl EnvironmentBoundary {
    pub(crate) fn into_call(self) -> Option<EnvironmentCall> {
        Some(EnvironmentCall {
            argument: self.argument?,
            span: self.span,
            begin: self.begin,
            direct: self.direct,
            operand: self.operand,
            deferred: self.deferred,
        })
    }
}

#[derive(Clone)]
pub(crate) struct SourceCall {
    pub command: syntax::Command,
    pub certain: bool,
}

impl<'a> CommandSignatures<'a> {
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.local.keys().map(|name| name.as_ref())
    }

    pub(crate) fn new(documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>) -> Self {
        Self::with_ambiguous_names(documents, std::iter::empty())
    }

    /// Paused declarations can exclude operands without certifying a meaning.
    pub(crate) fn with_literal_sources(
        mut self,
        documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>,
    ) -> Self {
        for (name, operands) in Self::new(documents).literal_operands {
            self.literal_operands
                .entry(name)
                .or_default()
                .extend(operands);
        }
        self
    }

    pub(crate) fn with_ambiguous_names(
        documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>,
        ambiguous: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::with_ambiguous_environments(documents, ambiguous, std::iter::empty())
    }

    pub(crate) fn with_ambiguous_environments(
        documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>,
        ambiguous: impl IntoIterator<Item = &'a str>,
        environments: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::collect(documents, ambiguous, environments, None)
    }

    pub(crate) fn source_context(
        documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>,
    ) -> Self {
        let documents: Vec<_> = documents.into_iter().collect();
        let preliminary = Self::new(documents.iter().copied());
        Self::collect(
            documents,
            std::iter::empty(),
            std::iter::empty(),
            Some(&preliminary),
        )
    }

    fn collect(
        documents: impl IntoIterator<Item = (&'a str, &'a syntax::Syntax)>,
        ambiguous: impl IntoIterator<Item = &'a str>,
        environments: impl IntoIterator<Item = &'a str>,
        source_context: Option<&Self>,
    ) -> Self {
        let documents: Vec<_> = documents.into_iter().collect();
        // Source-context filtering may omit definitions installed by a later
        // macro call. Never infer the primitive's arity from that filtered map.
        let mut raw_names = BTreeSet::new();
        let detokenize_boundary_known = documents.iter().any(|(_, parsed)| {
            parsed.commands.iter().any(|command| {
                matches!(command.name.as_str(), "detokenize" | "csuse" | "endcsname")
            })
        }) && !documents.iter().any(|(source, parsed)| {
            parsed.definitions.iter().any(|entry| {
                matches!(
                    entry.name.as_str(),
                    "detokenize" | "csuse" | "csname" | "endcsname"
                ) || !raw_names.insert(entry.name.as_str())
            }) || parsed.commands.iter().enumerate().any(|(index, command)| {
                let invoked = match command.name.as_str() {
                    "csuse" => command
                        .argument
                        .as_ref()
                        .filter(|argument| syntax::is_static_key(&argument.value))
                        .map(|argument| argument.value.as_str()),
                    "csname" => syntax::literal_csname(source, &parsed.commands, index)
                        .map(|span| &source[span]),
                    name => Some(name),
                };
                matches!(
                    invoked,
                    None | Some("@input" | "@input@" | "@@input" | "scantokens")
                ) || matches!(
                    command.name.as_str(),
                    "let" | "futurelet" | "cslet" | "letcs" | "csletcs"
                ) || syntax::code_input_command(&command.name)
                    || invoked.is_some_and(|name| {
                        name != command.name
                            && (matches!(
                                name,
                                "input" | "include" | "subfile" | "InputIfFileExists"
                            ) || syntax::loader_extension(name).is_some())
                    })
                    || (matches!(
                        command.name.as_str(),
                        "input" | "include" | "subfile" | "InputIfFileExists"
                    ) || syntax::loader_extension(&command.name).is_some())
                        && if syntax::file_list(&command.name) {
                            syntax::complete_file_list_spans(source, command).is_none()
                        } else {
                            syntax::file_argument(source, command).is_none_or(|argument| {
                                !syntax::is_literal_file_path(&argument.value)
                            })
                        }
            })
        });
        let mut local: BTreeMap<_, _> = ambiguous
            .into_iter()
            .map(|name| (Cow::Borrowed(name), None))
            .collect();
        for name in environments {
            local.insert(Cow::Borrowed(name), None);
            local.insert(Cow::Owned(format!("end{name}")), None);
        }
        let mut tex_operands: BTreeMap<_, _> =
            local.keys().map(|name| (name.clone(), None)).collect();
        let mut aliases = BTreeMap::new();
        let mut literal_operands = BTreeMap::new();
        for &(source, parsed) in &documents {
            let positions = source_context
                .map(|signatures| signatures.source_command_positions(source, parsed));
            for (entry, environment) in parsed
                .definitions
                .iter()
                .map(|entry| (entry, false))
                .chain(parsed.environments.iter().map(|entry| (entry, true)))
                .filter(|(entry, _)| {
                    positions.as_ref().is_none_or(|positions| {
                        let declaration = entry.detail.strip_prefix("Defined with \\");
                        let end = parsed
                            .commands
                            .partition_point(|command| command.span.start < entry.span.start);
                        parsed.commands[..end]
                            .iter()
                            .rev()
                            .find(|command| Some(command.name.as_str()) == declaration)
                            .is_some_and(|command| positions.contains(&command.span.start))
                    })
                })
            {
                // Environments and ordinary commands share their start control
                // in TeX. Duplicates must invalidate both calls and aliases.
                let mut signature = if environment {
                    environment_start_signature(source, parsed, entry)
                } else {
                    source_definition(source, parsed, entry, false)
                };
                let tex_definition = TexOperands::definition(source, parsed, entry);
                let tex_known = tex_definition.is_some();
                let tex = tex_definition
                    .filter(|(operands, _)| operands.delimited() || signature.is_none())
                    .map(|(operands, _)| operands);
                if TexOperands::declares(entry) && (tex.is_some() || !tex_known) {
                    // The help reader deliberately has fewer TeX token rules.
                    // Never fall back to its arity for unrecognized parameter text.
                    signature = None;
                }
                let bodies = environment
                    .then(|| environment_bodies(source, parsed, entry))
                    .flatten();
                let copied =
                    literal_copy_target(source, parsed, entry, environment).map(|(name, _)| name);
                let alias = signature
                    .as_ref()
                    .filter(|s| s.parameters.is_empty())
                    .and_then(|_| {
                        if environment {
                            bodies
                                .as_ref()
                                .and_then(|(begin, _)| body_alias(source, begin))
                        } else {
                            source_alias(source, parsed, entry)
                        }
                    });
                if environment {
                    let name = format!("end{}", entry.name);
                    let ending = signature
                        .as_ref()
                        .filter(|_| bodies.is_some() || entry.detail == "Defined with \\newtheorem")
                        .map(|_| Signature {
                            name: name.clone(),
                            parameters: vec![],
                            documentation: entry.detail.clone(),
                            starred: false,
                            nested_brackets: true,
                        });
                    let alias = ending
                        .as_ref()
                        .and_then(|_| bodies.as_ref().and_then(|(_, end)| body_alias(source, end)));
                    if let Some(target) = &copied {
                        literal_operands
                            .entry(Cow::Owned(name.clone()))
                            .or_insert_with(Vec::new)
                            .push(LiteralOperands::Copy(format!("end{target}")));
                    } else {
                        insert_literal_operands(
                            &mut literal_operands,
                            Cow::Owned(name.clone()),
                            ending.as_ref(),
                            None,
                            alias.as_deref(),
                        );
                    }
                    insert_command_signature(
                        &mut local,
                        &mut tex_operands,
                        &mut aliases,
                        Cow::Owned(name),
                        ending,
                        None,
                        alias,
                    );
                }
                // Expanded definitions capture a meaning at definition time;
                // their lexical body cannot establish a late-bound alias.
                let expanded = matches!(
                    entry.detail.strip_prefix("Defined with \\"),
                    Some("edef" | "xdef" | "csedef" | "csxdef")
                );
                if expanded && alias.is_some() {
                    signature = None;
                }
                if let Some(target) = copied {
                    literal_operands
                        .entry(Cow::Borrowed(entry.name.as_str()))
                        .or_insert_with(Vec::new)
                        .push(LiteralOperands::Copy(target));
                } else {
                    insert_literal_operands(
                        &mut literal_operands,
                        Cow::Borrowed(entry.name.as_str()),
                        signature.as_ref(),
                        tex.as_ref(),
                        alias.as_deref().filter(|_| !expanded),
                    );
                }
                insert_command_signature(
                    &mut local,
                    &mut tex_operands,
                    &mut aliases,
                    Cow::Borrowed(entry.name.as_str()),
                    signature,
                    tex,
                    alias.filter(|_| !expanded),
                );
            }
        }
        local.extend(resolve_aliases(&local, &aliases, operand_builtin));
        tex_operands.extend(resolve_aliases(&tex_operands, &aliases, |_| None));
        let mut signatures = Self {
            local,
            tex_operands,
            literal_operands,
            detokenize_boundary_known,
        };
        // Deferred or forwarded providers can install an unseen definition.
        // Proven displayed file commands cannot execute such a provider.
        if signatures.detokenize_boundary_known
            && documents.iter().any(|(source, parsed)| {
                let active = signatures.source_command_positions(source, parsed);
                let quoted = signatures.quoted_command_tokens(source, parsed);
                parsed.commands.iter().any(|command| {
                    (matches!(
                        command.name.as_str(),
                        "input" | "include" | "subfile" | "InputIfFileExists"
                    ) || syntax::loader_extension(&command.name).is_some())
                        && !active.contains(&command.span.start)
                        && !quoted.contains(&command.span.start)
                })
            })
        {
            signatures.detokenize_boundary_known = false;
        }
        signatures
    }

    fn invocation<'b>(
        &self,
        source: &'b str,
        parsed: &'b syntax::Syntax,
        index: usize,
        operands: &BTreeSet<usize>,
        declarations: &BTreeSet<(usize, usize)>,
    ) -> Option<(&'b str, usize, usize, bool)> {
        let command = parsed.commands.get(index)?;
        let local = &self.local;
        // Grouped csuse names leave following spaces intact; a literal
        // endcsname, like an ordinary control word, discards lexical blanks.
        let named = command.argument.as_ref().filter(|argument| {
            command.name == "csuse"
                && !local.contains_key("csuse")
                && syntax::is_static_key(&argument.value)
        });
        let literal = (command.name == "endcsname"
            && !local.contains_key("csname")
            && !local.contains_key("endcsname"))
        .then(|| {
            index.checked_sub(1).and_then(|index| {
                (!operands.contains(&parsed.commands[index].span.start))
                    .then(|| syntax::literal_csname(source, &parsed.commands, index))
                    .flatten()
            })
        })
        .flatten();
        if literal
            .as_ref()
            .is_some_and(|span| declarations.contains(&(span.start, span.end)))
        {
            return None;
        }
        let name = named
            .map(|argument| argument.value.as_str())
            .or_else(|| literal.as_ref().map(|span| &source[span.clone()]))
            .unwrap_or(command.name.as_str());
        let end = named.map_or(command.span.end, |argument| argument.outer_span.end);
        let start = literal.as_ref().map_or(command.span.start, |_| {
            parsed.commands[index - 1].span.start
        });
        Some((name, start, end, named.is_none()))
    }

    pub(crate) fn invokes_builtin(&self, name: &str, builtin: &str) -> bool {
        !self.local.contains_key(builtin)
            && self.local.get(name).map_or(name == builtin, |signature| {
                signature.as_ref().is_some_and(|s| s.name == builtin)
            })
    }

    pub(crate) fn literal_integer_value(&self, source: &str, after: usize) -> Option<(i32, usize)> {
        syntax::literal_integer(source, after, |name| {
            [
                "relax",
                "global",
                "globaldefs",
                "DeclareGraphicsExtensions",
                "graphicspath",
                "begingroup",
                "endgroup",
                "bgroup",
                "egroup",
            ]
            .iter()
            .any(|builtin| self.invokes_builtin(name, builtin))
        })
    }

    pub(crate) fn builtin_alias_body_positions(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        builtins: &[&str],
    ) -> BTreeSet<usize> {
        parsed
            .definitions
            .iter()
            .filter(|entry| {
                builtins
                    .iter()
                    .any(|builtin| self.invokes_builtin(&entry.name, builtin))
            })
            .filter_map(|entry| source_alias_body(source, parsed, entry))
            .map(|body| syntax::skip_space(source, body.span.start))
            .collect()
    }

    pub(crate) fn environment_calls(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> Vec<(syntax::Argument, bool)> {
        self.environment_invocations(source, parsed)
            .into_iter()
            .filter(|call| !call.direct)
            .map(|call| (call.argument, call.operand))
            .collect()
    }

    pub(crate) fn environment_primitives_unchanged(&self) -> bool {
        !self.local.contains_key("begin") && !self.local.contains_key("end")
    }

    pub(crate) fn command_unchanged(&self, name: &str) -> bool {
        !self.local.contains_key(name)
    }

    /// Literal filenames following static named calls and pure source aliases.
    /// Ordinary input/include commands retain their existing lexical handling.
    pub(crate) fn indirect_source_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(
            source,
            parsed,
            positions,
            &["input", "include", "InputIfFileExists"],
        )
    }

    /// Static package and class loaders share source-call token boundaries, including
    /// options and comma-separated names, without executing macro bodies.
    pub(crate) fn indirect_loader_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(
            source,
            parsed,
            positions,
            &[
                "usepackage",
                "RequirePackage",
                "RequirePackageWithOptions",
                "documentclass",
                "LoadClass",
                "LoadClassWithOptions",
            ],
        )
    }

    /// Option forwarding refers to a file without importing its declarations.
    pub(crate) fn indirect_option_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(
            source,
            parsed,
            positions,
            &["PassOptionsToPackage", "PassOptionsToClass"],
        )
    }

    /// A complete static selection scope, with unknown calls kept distinct
    /// from an absence of filters. Pure alias bodies are declarations, not calls.
    pub(crate) fn selection_filters(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> Option<Vec<BTreeSet<String>>> {
        if ["includeonly", "csuse", "csname", "endcsname"]
            .iter()
            .any(|name| self.local.contains_key(*name))
        {
            return None;
        }
        let positions = self.source_command_positions(source, parsed);
        let declarations: BTreeSet<_> = parsed
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let alias_bodies: BTreeSet<_> = parsed
            .definitions
            .iter()
            .filter(|entry| self.invokes_builtin(&entry.name, "includeonly"))
            .filter_map(|entry| source_alias_body(source, parsed, entry))
            .map(|body| syntax::skip_space(source, body.span.start))
            .collect();
        let empty_operands = BTreeSet::new();
        for (index, command) in parsed.commands.iter().enumerate() {
            if declarations.contains(&(command.span.start, command.span.end)) {
                continue;
            }
            if positions.contains(&command.span.start) {
                if command.name == "csuse"
                    && command
                        .argument
                        .as_ref()
                        .is_none_or(|argument| !syntax::is_static_key(&argument.value))
                {
                    return None;
                }
                if command.name == "csname"
                    && syntax::literal_csname(source, &parsed.commands, index).is_none()
                {
                    return None;
                }
            }
            let Some((name, start, _, _)) =
                self.invocation(source, parsed, index, &empty_operands, &declarations)
            else {
                continue;
            };
            if self.invokes_builtin(name, "includeonly")
                && !positions.contains(&start)
                && !alias_bodies.contains(&start)
            {
                // An unsupported wrapper or operand can execute a selector.
                // Its presence cannot certify that the document has no filter.
                return None;
            }
        }
        let indirect = self.indirect_selection_invocations(source, parsed, &positions);
        if indirect.iter().any(|call| !call.certain) {
            return None;
        }
        parsed
            .commands
            .iter()
            .filter(|command| {
                command.name == "includeonly" && positions.contains(&command.span.start)
            })
            .chain(indirect.iter().map(|call| &call.command))
            .map(|command| {
                syntax::includeonly_names(source, command).map(|names| names.into_iter().collect())
            })
            .collect()
    }

    /// Existence checks reference files without executing their contents.
    pub(crate) fn indirect_conditional_checks(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(source, parsed, positions, &["IfFileExists"])
    }

    /// Selection references name chapters without loading their declarations.
    pub(crate) fn indirect_selection_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(source, parsed, positions, &["includeonly"])
    }

    /// Bibliography resources provide data rather than LaTeX declarations.
    pub(crate) fn indirect_bibliography_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(
            source,
            parsed,
            positions,
            &[
                "bibliography",
                "nobibliography",
                "bibliographystyle",
                "addbibresource",
                "addglobalbib",
            ],
        )
    }

    pub(crate) fn indirect_graphics_path_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(source, parsed, positions, &["graphicspath"])
    }

    pub(crate) fn indirect_graphics_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        self.indirect_file_invocations(source, parsed, positions, &["includegraphics"])
    }

    pub(crate) fn graphics_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
    ) -> Vec<SourceCall> {
        let mut calls = self.indirect_graphics_invocations(source, parsed, positions);
        calls.extend(
            parsed
                .commands
                .iter()
                .filter(|command| {
                    command.name == "includegraphics" && positions.contains(&command.span.start)
                })
                .map(|command| SourceCall {
                    command: command.clone(),
                    certain: self.command_unchanged("includegraphics"),
                }),
        );
        for call in &mut calls {
            call.command.argument = crate::graphics::argument(source, &call.command);
        }
        calls.sort_by_key(|call| call.command.span.start);
        calls
    }

    fn indirect_file_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        positions: &BTreeSet<usize>,
        builtins: &[&str],
    ) -> Vec<SourceCall> {
        let aliased = self
            .local
            .values()
            .flatten()
            .any(|signature| builtins.contains(&signature.name.as_str()));
        if !aliased
            && !parsed
                .commands
                .iter()
                .any(|command| matches!(command.name.as_str(), "csuse" | "endcsname"))
        {
            return vec![];
        }
        let operands = parsed
            .commands
            .iter()
            .filter(|command| !positions.contains(&command.span.start))
            .map(|command| command.span.start)
            .collect();
        let declarations: BTreeSet<_> = parsed
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        parsed
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                if builtins.contains(&command.name.as_str())
                    || declarations.contains(&(command.span.start, command.span.end))
                {
                    return None;
                }
                let (name, start, end, _) =
                    self.invocation(source, parsed, index, &operands, &declarations)?;
                if !positions.contains(&start) || !positions.contains(&command.span.start) {
                    return None;
                }
                let builtin = builtins.iter().copied().find(|builtin| {
                    name == *builtin
                        || self
                            .local
                            .get(name)
                            .and_then(Option::as_ref)
                            .is_some_and(|signature| signature.name == *builtin)
                })?;
                Some(SourceCall {
                    command: syntax::Command {
                        name: builtin.into(),
                        span: start..end,
                        argument: if syntax::loader_extension(builtin).is_some()
                            || matches!(builtin, "bibliographystyle" | "includegraphics")
                            || crate::bib_resources::loads(builtin)
                        {
                            syntax::file_name_argument_start(source, builtin, end)
                                .and_then(|start| {
                                    if crate::bib_resources::loads(builtin) {
                                        syntax::verbatim_file_group(source, start)
                                    } else {
                                        syntax::group(source, start, b'{', b'}')
                                    }
                                })
                                .map(|(argument, _)| argument)
                        } else {
                            syntax::source_input_argument(source, builtin, end)
                        },
                    },
                    certain: self.invokes_builtin(name, builtin),
                })
            })
            .collect()
    }

    pub(crate) fn source_command_positions(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> BTreeSet<usize> {
        let operands = self.argument_tokens(source, parsed);
        let bodies = definition_body_spans(source, parsed);
        let active =
            |start| !operands.contains(&start) && !bodies.iter().any(|span| span.contains(&start));
        // A redefined endinput also ends the region we can interpret safely.
        let stop = parsed
            .commands
            .iter()
            .filter(|command| command.name == "endinput" && active(command.span.start))
            .map(|command| {
                source[command.span.end..]
                    .find(['\r', '\n'])
                    .map_or(source.len(), |after| command.span.end + after)
            })
            .min()
            .unwrap_or(source.len());
        parsed
            .commands
            .iter()
            .filter(|command| command.span.start < stop && active(command.span.start))
            .map(|command| command.span.start)
            .collect()
    }

    pub(crate) fn environment_invocations(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> Vec<EnvironmentCall> {
        self.environment_boundaries(source, parsed)
            .into_iter()
            .filter_map(EnvironmentBoundary::into_call)
            .collect()
    }

    pub(crate) fn environment_boundaries(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> Vec<EnvironmentBoundary> {
        let operands = self.argument_tokens(source, parsed);
        let bodies = definition_body_spans(source, parsed);
        let declarations = parsed
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        parsed
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                // Retain potential names inside data for rename validation;
                // they must never become editable environment references.
                let (name, start, end, _) =
                    self.invocation(source, parsed, index, &BTreeSet::new(), &declarations)?;
                if !self.invokes_builtin(name, "begin") && !self.invokes_builtin(name, "end") {
                    return None;
                }
                let argument = syntax::environment_name_argument(source, end);
                Some(EnvironmentBoundary {
                    span: start..argument.as_ref().map_or(end, |arg| arg.outer_span.end),
                    begin: self.invokes_builtin(name, "begin"),
                    direct: matches!(command.name.as_str(), "begin" | "end"),
                    operand: operands.contains(&start) || operands.contains(&command.span.start),
                    deferred: bodies.iter().any(|span| span.contains(&start)),
                    argument,
                })
            })
            .collect()
    }

    /// Tokens definitely rendered as characters by an unchanged TeX primitive.
    /// Unknown user macros can execute their operands, so their argument tokens
    /// are insufficient evidence for removing a build dependency.
    pub(crate) fn quoted_command_tokens(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> BTreeSet<usize> {
        let operands = self.argument_tokens(source, parsed);
        let declarations = parsed
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let mut result = BTreeSet::new();
        for (index, command) in parsed.commands.iter().enumerate() {
            if operands.contains(&command.span.start) {
                continue;
            }
            let Some((name, _, end, control_spaces)) =
                self.invocation(source, parsed, index, &operands, &declarations)
            else {
                continue;
            };
            let at = optional_start(source, end, control_spaces);
            if self.invokes_builtin(name, "detokenize") {
                if let Some((argument, end)) = syntax::group(source, at, b'{', b'}') {
                    mark_operand_commands(parsed, argument.span.start, end, &mut result);
                }
            } else if self.invokes_builtin(name, "string") || self.invokes_builtin(name, "meaning")
            {
                // These primitives consume one token, including an opening
                // brace itself; they do not consume a following whole group.
                if required_token(source, at).is_some()
                    && syntax::control_word(source, at).is_some()
                {
                    result.insert(at);
                }
            }
        }
        result
    }

    pub(crate) fn argument_tokens(&self, source: &str, parsed: &syntax::Syntax) -> BTreeSet<usize> {
        self.argument_tokens_with_literal(source, parsed, None, false, false)
    }

    /// Positions confirmed by a known parameter grammar, excluding the
    /// conservative operand assumed after an unresolved or ambiguous macro.
    pub(crate) fn known_argument_tokens(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
    ) -> BTreeSet<usize> {
        self.argument_tokens_with_literal(source, parsed, None, false, true)
    }

    pub(crate) fn literal_is_argument(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        at: usize,
    ) -> bool {
        self.argument_tokens_with_literal(source, parsed, Some(at), false, false)
            .contains(&at)
            || self.possible_literal_argument(source, parsed, at)
    }

    /// A possible declaration can exclude a literal without assigning unknown
    /// commands in the consumer the declaration file's fallback arity.
    pub(crate) fn declared_literal_is_argument(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        at: usize,
    ) -> bool {
        self.argument_tokens_with_literal(source, parsed, Some(at), true, false)
            .contains(&at)
            || self.possible_literal_argument(source, parsed, at)
    }

    fn possible_literal_argument(&self, source: &str, parsed: &syntax::Syntax, at: usize) -> bool {
        if self.literal_operands.is_empty() {
            return false;
        }
        let declarations: BTreeSet<_> = parsed
            .definitions
            .iter()
            .chain(&parsed.environments)
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let mut deferred = definition_body_spans(source, parsed);
        for entry in &parsed.definitions {
            if let Some((_, end)) = TexOperands::definition(source, parsed, entry) {
                deferred.push(entry.span.start..end);
            }
        }
        for (entry, environment) in parsed
            .definitions
            .iter()
            .map(|entry| (entry, false))
            .chain(parsed.environments.iter().map(|entry| (entry, true)))
        {
            if !self
                .local
                .contains_key(entry.detail.strip_prefix("Defined with \\").unwrap_or(""))
            {
                if let Some((_, end)) = literal_copy_target(source, parsed, entry, environment) {
                    deferred.push(entry.span.start..end);
                }
            }
        }
        let empty = BTreeSet::new();
        for (index, command) in parsed
            .commands
            .iter()
            .enumerate()
            .take_while(|(_, command)| command.span.start < at)
        {
            if parsed.is_ignored(command.span.start)
                || declarations.contains(&(command.span.start, command.span.end))
                || deferred
                    .iter()
                    .any(|span| span.contains(&command.span.start))
            {
                continue;
            }
            let Some((name, _, after, spaces)) =
                self.invocation(source, parsed, index, &empty, &declarations)
            else {
                continue;
            };
            if !self.literal_operands.contains_key(name)
                && (name != "begin" || self.local.contains_key("begin"))
            {
                continue;
            }
            let mut pending = vec![(name.to_owned(), after, spaces, false)];
            let mut visited = BTreeSet::new();
            // Do not skip calls merely consumed by another possible arity:
            // a different declaration may leave that call executable.
            while let Some((name, after, spaces, copied)) = pending.pop() {
                if !visited.insert((name.clone(), after, spaces, copied)) {
                    continue;
                }
                let declared = self.literal_operands.contains_key(name.as_str());
                if name == "begin" && (copied || !self.local.contains_key("begin")) {
                    if let Some(argument) = syntax::environment_name_argument(source, after) {
                        if self.literal_operands.contains_key(argument.value.as_str()) {
                            pending.push((argument.value, argument.outer_span.end, false, false));
                        }
                    }
                    if !declared {
                        continue;
                    }
                }
                let mut consumed = BTreeSet::new();
                if copied || !declared {
                    if name == "halign" && (copied || !self.local.contains_key("halign")) {
                        // A captured primitive alignment executes its body.
                        if !declared {
                            continue;
                        }
                    } else if let Some(signature) = operand_builtin(&name) {
                        let opaque = matches!(
                            name.as_str(),
                            "detokenize"
                                | "graphicspath"
                                | "DeclareGraphicsExtensions"
                                | "lstset"
                                | "fvset"
                                | "includeonly"
                        );
                        argument_tokens(
                            (source, parsed),
                            after,
                            spaces,
                            &signature,
                            opaque,
                            Some(at),
                            &mut consumed,
                        );
                    } else if !declared {
                        unknown_argument_token(source, after, parsed, Some(at), &mut consumed);
                    }
                    if consumed.contains(&at) {
                        return true;
                    }
                }
                for operands in self
                    .literal_operands
                    .get(name.as_str())
                    .into_iter()
                    .flatten()
                {
                    match operands {
                        LiteralOperands::Alias(target) => {
                            pending.push((target.clone(), after, spaces, false))
                        }
                        LiteralOperands::Copy(target) => {
                            pending.push((target.clone(), after, spaces, true))
                        }
                        LiteralOperands::Tex(operands) => {
                            if operands
                                .end(source, after, spaces)
                                .is_some_and(|end| (after..end).contains(&at))
                            {
                                return true;
                            }
                        }
                        LiteralOperands::Signature(signature) => {
                            argument_tokens(
                                (source, parsed),
                                after,
                                spaces,
                                signature,
                                true,
                                Some(at),
                                &mut consumed,
                            );
                            if consumed.contains(&at) {
                                return true;
                            }
                        }
                        LiteralOperands::Unknown => {
                            unknown_argument_token(source, after, parsed, Some(at), &mut consumed);
                            if consumed.contains(&at) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn argument_tokens_with_literal(
        &self,
        source: &str,
        parsed: &syntax::Syntax,
        literal: Option<usize>,
        declared_only: bool,
        known_only: bool,
    ) -> BTreeSet<usize> {
        let local = &self.local;
        let mut result = BTreeSet::new();
        let declarations: BTreeSet<_> = parsed
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        for entry in &parsed.definitions {
            if let Some((_, end)) = TexOperands::definition(source, parsed, entry) {
                // Parameter-text delimiters are data at declaration time.
                // In particular a delimiter control must
                // not consume the first invocation following the definition.
                mark_operand_tokens(parsed, entry.span.start, end, literal, &mut result);
            }
        }
        for (entry, environment) in parsed
            .definitions
            .iter()
            .map(|entry| (entry, false))
            .chain(parsed.environments.iter().map(|entry| (entry, true)))
        {
            if !self
                .local
                .contains_key(entry.detail.strip_prefix("Defined with \\").unwrap_or(""))
            {
                if let Some((_, end)) = literal_copy_target(source, parsed, entry, environment) {
                    mark_operand_tokens(parsed, entry.span.start, end, literal, &mut result);
                }
            }
        }
        for (index, command) in parsed.commands.iter().enumerate() {
            if result.contains(&command.span.start) {
                continue;
            }
            if let Some(shape) = parsed.contextual_command_shapes.get(&command.span.start) {
                shape.argument_tokens(source, parsed, command, literal, &mut result);
                continue;
            }
            if let Some(arity) = parsed.string_test_arities.get(&command.span.start) {
                if let Some(count) = arity {
                    let name = if *count == 2 { "ifstrequal" } else { "ifblank" };
                    if let Some(signature) = operand_builtin(name) {
                        argument_tokens(
                            (source, parsed),
                            command.span.end,
                            true,
                            &signature,
                            false,
                            literal,
                            &mut result,
                        );
                    }
                } else if !known_only {
                    unknown_argument_token(source, command.span.end, parsed, literal, &mut result);
                }
                continue;
            }
            let Some((name, _, end, control_spaces)) =
                self.invocation(source, parsed, index, &result, &declarations)
            else {
                continue;
            };
            if declared_only && !local.contains_key(name) {
                continue;
            }
            // A primitive alignment executes its body. Treating that body as
            // opaque macro data would hide the literal row tabs we validate.
            if literal.is_some() && self.invokes_builtin(name, "halign") {
                continue;
            }
            if self.invokes_builtin(name, "globaldefs") {
                if let Some((_, after)) = self.literal_integer_value(source, end) {
                    // A backtick control is a character-code operand, not a
                    // macro invocation that can consume the following call.
                    mark_operand_tokens(parsed, end, after, literal, &mut result);
                }
                continue;
            }
            // This primitive consumes exactly one balanced general-text
            // group. The unknown-macro fallback would also consume the next
            // control token after that group, hiding an independent file call.
            if self.invokes_builtin(name, "detokenize") {
                if !self.detokenize_boundary_known {
                    if !known_only {
                        unknown_argument_token(source, end, parsed, literal, &mut result);
                    }
                    continue;
                }
                let at = syntax::skip_space(source, end);
                if let Some((_, after)) = syntax::group(source, at, b'{', b'}') {
                    mark_operand_tokens(parsed, at, after, literal, &mut result);
                } else if !known_only {
                    unknown_argument_token(source, end, parsed, literal, &mut result);
                }
                continue;
            }
            // A named call or a pure alias can invoke begin. Its environment
            // name follows the resolved invocation, rather than the lexical
            // command (whose first group may instead contain "begin").
            let begin = self
                .invokes_builtin(name, "begin")
                .then(|| syntax::environment_name_argument(source, end))
                .flatten();
            let environment = begin
                .as_ref()
                .filter(|argument| local.contains_key(argument.value.as_str()));
            let name = environment.map_or(name, |argument| argument.value.as_str());
            let end = environment.map_or(end, |argument| argument.outer_span.end);
            let control_spaces = environment.is_none() && control_spaces;
            if let Some(end) = self
                .tex_operands
                .get(name)
                .and_then(Option::as_ref)
                .and_then(|operands| operands.end(source, end, control_spaces))
            {
                // Delimited TeX arguments are read without executing any of
                // their tokens, including controls inside balanced groups.
                mark_operand_tokens(parsed, command.span.end, end, literal, &mut result);
                continue;
            }
            let signature = local.get(name).and_then(Option::as_ref);
            let builtin = begin
                .as_ref()
                .filter(|_| environment.is_none())
                .and_then(|argument| builtin_environment(&argument.value))
                .or_else(|| {
                    (!local.contains_key(name))
                        .then(|| operand_builtin(name))
                        .flatten()
                });
            // A builtin environment has additional parameters beyond begin's
            // name group, including when begin was reached through an alias.
            if let Some(signature) = builtin.as_ref().or(signature) {
                // Resolved aliases retain their leaf's name. Builtin
                // formatters and their pure aliases execute grouped contents;
                // a user macro can instead discard or detokenize its argument.
                let opaque = local.contains_key(signature.name.as_str())
                    || matches!(
                        signature.name.as_str(),
                        "graphicspath"
                            | "DeclareGraphicsExtensions"
                            | "lstset"
                            | "fvset"
                            | "includeonly"
                    );
                argument_tokens(
                    (source, parsed),
                    end,
                    control_spaces,
                    signature,
                    opaque,
                    literal,
                    &mut result,
                );
            } else if !known_only
                && (literal.is_none() || !self.literal_operands.contains_key(name))
            {
                unknown_argument_token(source, end, parsed, literal, &mut result);
            }
        }
        result
    }
}

fn insert_command_signature<'a>(
    local: &mut BTreeMap<Cow<'a, str>, Option<Signature>>,
    tex_operands: &mut BTreeMap<Cow<'a, str>, Option<TexOperands>>,
    aliases: &mut BTreeMap<Cow<'a, str>, String>,
    name: Cow<'a, str>,
    signature: Option<Signature>,
    tex: Option<TexOperands>,
    alias: Option<String>,
) {
    match local.entry(name) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            if let Some(target) = alias {
                aliases.insert(entry.key().clone(), target);
            }
            tex_operands.insert(entry.key().clone(), tex);
            entry.insert(signature);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            aliases.remove(entry.key());
            tex_operands.insert(entry.key().clone(), None);
            entry.insert(None);
        }
    }
}

fn argument_free(name: &str) -> bool {
    // Standard math symbols and operators emit an atom without consuming the
    // next control token. In particular, alpha/right and sin/right are two
    // independent calls. Local declarations still override these builtins.
    matches!(
        name,
        "relax"
            | "noindent"
            | "indent"
            | "normalfont"
            | "selectfont"
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
            | "global"
            | "globaldefs"
            | "long"
            | "outer"
            | "protected"
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
            | "alpha"
            | "beta"
            | "gamma"
            | "delta"
            | "epsilon"
            | "varepsilon"
            | "zeta"
            | "eta"
            | "theta"
            | "vartheta"
            | "iota"
            | "kappa"
            | "lambda"
            | "mu"
            | "nu"
            | "xi"
            | "pi"
            | "varpi"
            | "rho"
            | "varrho"
            | "sigma"
            | "varsigma"
            | "tau"
            | "upsilon"
            | "phi"
            | "varphi"
            | "chi"
            | "psi"
            | "omega"
            | "Gamma"
            | "Delta"
            | "Theta"
            | "Lambda"
            | "Xi"
            | "Pi"
            | "Sigma"
            | "Upsilon"
            | "Phi"
            | "Psi"
            | "Omega"
            | "infty"
            | "partial"
            | "nabla"
            | "ell"
            | "hbar"
            | "imath"
            | "jmath"
            | "sum"
            | "prod"
            | "coprod"
            | "int"
            | "oint"
            | "bigcup"
            | "bigcap"
            | "bigsqcup"
            | "bigvee"
            | "bigwedge"
            | "bigodot"
            | "bigotimes"
            | "bigoplus"
            | "biguplus"
            | "log"
            | "lg"
            | "ln"
            | "lim"
            | "limsup"
            | "liminf"
            | "sin"
            | "arcsin"
            | "sinh"
            | "cos"
            | "arccos"
            | "cosh"
            | "tan"
            | "arctan"
            | "tanh"
            | "cot"
            | "coth"
            | "sec"
            | "csc"
            | "max"
            | "min"
            | "sup"
            | "inf"
            | "arg"
            | "ker"
            | "dim"
            | "hom"
            | "det"
            | "exp"
            | "Pr"
            | "gcd"
            | "deg"
    )
}

fn operand_builtin(name: &str) -> Option<Signature> {
    builtin(name).or_else(|| {
        let parameters = if argument_free(name) {
            vec![]
        } else if name == "detokenize" {
            // Give pure aliases the primitive's grouped operand boundary.
            vec![Parameter {
                obey_spaces: false,
                kind: Kind::RequiredGroup,
                label: "{tokens}".into(),
            }]
        } else if name == "newtheorem" {
            // Both numbered forms end after the title and possible reset
            // counter; the starred form has no optional counter operands.
            [
                (Kind::Required, "{name}"),
                (Kind::UnstarredOptional, "[shared]"),
                (Kind::Required, "{title}"),
                (Kind::UnstarredOptional, "[within]"),
            ]
            .into_iter()
            .map(|(kind, label)| Parameter {
                obey_spaces: false,
                kind,
                label: label.into(),
            })
            .collect()
        } else if matches!(
            name,
            "graphicspath" | "DeclareGraphicsExtensions" | "lstset" | "fvset" | "includeonly"
        ) {
            // One configuration argument ends here; a following file call
            // is independent. Settings remain data, including through aliases.
            vec![Parameter {
                obey_spaces: false,
                kind: Kind::Required,
                label: "{configuration}".into(),
            }]
        } else if matches!(name, "csdef" | "csgdef" | "csedef" | "csxdef") {
            // Grouped destinations and replacement text do not consume the
            // following command. Parameter text stops this group-only reader.
            ["{name}", "{definition}"]
                .into_iter()
                .map(|label| Parameter {
                    obey_spaces: false,
                    kind: Kind::RequiredGroup,
                    label: label.into(),
                })
                .collect()
        } else {
            return None;
        };
        Some(Signature {
            name: name.into(),
            parameters,
            documentation: String::new(),
            starred: name == "newtheorem",
            nested_brackets: name != "newtheorem",
        })
    })
}

/// Braces in arguments and replacement text are balanced before their tokens
/// can execute. Include unfinished groups so diagnostics still report them.
pub(crate) fn reads_brace(source: &str, command: &syntax::Command, brace: usize) -> bool {
    fn operand_end(source: &str, mut at: usize) -> Option<usize> {
        let value = token(source, at)?;
        at += value.len();
        if value == r"\csname" {
            loop {
                at = syntax::skip_space(source, at);
                let value = token(source, at)?;
                at += value.len();
                if value == r"\endcsname" {
                    break;
                }
            }
        }
        Some(syntax::skip_space(source, at))
    }

    let mut at = syntax::skip_space(source, command.span.end);
    if matches!(
        command.name.as_str(),
        "def" | "gdef" | "edef" | "xdef" | "csdef" | "csgdef" | "csedef" | "csxdef"
    ) {
        if command.name.starts_with("cs") {
            if let Some((_, end)) = syntax::group(source, at, b'{', b'}') {
                if brace < end {
                    return true;
                }
                at = syntax::skip_space(source, end);
            }
        }
        while at <= brace {
            if source.as_bytes().get(at) == Some(&b'{') {
                return syntax::group(source, at, b'{', b'}').is_none_or(|(_, end)| brace < end);
            }
            let Some(end) = operand_end(source, at) else {
                return false;
            };
            at = end;
        }
        return false;
    }
    let Some(signature) = operand_builtin(&command.name) else {
        // Unknown arity: retain lexical pairing within a following group.
        while at < brace {
            let (open, close) = match source.as_bytes().get(at) {
                Some(b'{') => (b'{', b'}'),
                Some(b'[') => (b'[', b']'),
                _ => return false,
            };
            let Some((_, end)) = syntax::group(source, at, open, close) else {
                return true;
            };
            if brace < end {
                return true;
            }
            at = syntax::skip_space(source, end);
        }
        return false;
    };
    if signature.starred && source.as_bytes().get(at) == Some(&b'*') {
        at = syntax::skip_space(source, at + 1);
    }
    for parameter in &signature.parameters {
        if at >= brace {
            return false;
        }
        let bracket = match &parameter.kind {
            Kind::Optional | Kind::UnstarredOptional | Kind::RequiredBracket => {
                if source.as_bytes().get(at) != Some(&b'[') {
                    if matches!(parameter.kind, Kind::RequiredBracket) {
                        return false;
                    }
                    continue;
                }
                true
            }
            Kind::Star => {
                if source.as_bytes().get(at) == Some(&b'*') {
                    at = syntax::skip_space(source, at + 1);
                }
                continue;
            }
            Kind::Flag(_) | Kind::Delimited { .. } => return false,
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
            let Some((_, end)) = group else {
                return true;
            };
            if brace < end {
                return true;
            }
            at = syntax::skip_space(source, end);
        } else {
            if matches!(parameter.kind, Kind::RequiredGroup) {
                return false;
            }
            let Some(end) = required_token(source, at).and_then(|_| operand_end(source, at)) else {
                return false;
            };
            at = end;
        }
    }
    false
}

/// Walk and memoize alias chains iteratively, including unknown targets and
/// cycles. The source graph may be arbitrarily deep without using Rust's stack.
fn resolve_aliases<'a, T: Clone>(
    local: &BTreeMap<Cow<'a, str>, Option<T>>,
    aliases: &BTreeMap<Cow<'a, str>, String>,
    builtin: impl Fn(&str) -> Option<T>,
) -> BTreeMap<Cow<'a, str>, Option<T>> {
    let mut resolved: BTreeMap<Cow<'a, str>, Option<T>> = BTreeMap::new();
    for name in aliases.keys() {
        let mut at = name.as_ref();
        let mut chain = vec![];
        let mut seen = BTreeSet::new();
        let signature = loop {
            if let Some(signature) = resolved.get(at) {
                break signature.clone();
            }
            if !seen.insert(at) {
                break None;
            }
            if let Some((key, target)) = aliases.get_key_value(at) {
                chain.push(key.clone());
                at = target;
            } else {
                break local.get(at).cloned().unwrap_or_else(|| builtin(at));
            }
        };
        for name in chain {
            resolved.insert(name, signature.clone());
        }
    }
    resolved
}

fn mark_operand_commands(
    parsed: &syntax::Syntax,
    start: usize,
    end: usize,
    result: &mut BTreeSet<usize>,
) {
    let first = parsed
        .commands
        .partition_point(|command| command.span.start < start);
    result.extend(
        parsed.commands[first..]
            .iter()
            .take_while(|command| command.span.start < end)
            .map(|command| command.span.start),
    );
}

fn mark_operand_tokens(
    parsed: &syntax::Syntax,
    start: usize,
    end: usize,
    literal: Option<usize>,
    result: &mut BTreeSet<usize>,
) {
    mark_operand_commands(parsed, start, end, result);
    if let Some(at) = literal.filter(|at| (start..end).contains(at)) {
        result.insert(at);
    }
}

fn unknown_argument_token(
    source: &str,
    mut at: usize,
    parsed: &syntax::Syntax,
    literal: Option<usize>,
    result: &mut BTreeSet<usize>,
) {
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
            mark_operand_tokens(parsed, at, end, literal, result);
            at = end;
        } else {
            if syntax::control_word(source, at).is_some() || literal == Some(at) {
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

fn environment_declaration_operand(name: &str, parameter: usize) -> bool {
    match name {
        "newenvironment" | "renewenvironment" | "provideenvironment" | "NewEnviron"
        | "RenewEnviron" => parameter < 3,
        "NewDocumentEnvironment"
        | "RenewDocumentEnvironment"
        | "ProvideDocumentEnvironment"
        | "DeclareDocumentEnvironment" => parameter < 2,
        "newtheorem" => parameter != 2,
        name if syntax::declares_environment_copy(name) => true,
        _ => false,
    }
}

fn argument_tokens(
    document: (&str, &syntax::Syntax),
    mut at: usize,
    mut control_spaces: bool,
    signature: &Signature,
    opaque: bool,
    literal: Option<usize>,
    result: &mut BTreeSet<usize>,
) {
    let (source, parsed) = document;
    let mut starred = false;
    if signature.starred {
        at = optional_start(source, at, true);
        if source.as_bytes().get(at) == Some(&b'*') {
            at += 1;
            starred = true;
            control_spaces = false;
        }
    }
    for (index, parameter) in signature.parameters.iter().enumerate() {
        // Defaults and parameter specifications are data; start/end code is
        // indexed normally. A body may display its defaults without executing
        // their tokens, so color renames must not rewrite those tokens.
        let opaque =
            (opaque || environment_declaration_operand(&signature.name, index)).then_some(parsed);
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
                if let Some(parsed) = opaque {
                    mark_operand_tokens(parsed, at, end, literal, result);
                }
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
            if let Some(parsed) = opaque {
                mark_operand_tokens(parsed, at, end, literal, result);
            }
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
            if word || literal == Some(at) {
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
    lookup(workspace, name, false, scope, None)
}

pub(crate) fn uses_builtin(workspace: &Workspace, name: &str, scope: &BTreeSet<Url>) -> bool {
    builtin(name)
        .is_some_and(|builtin| signature(workspace, name, scope).as_ref() == Some(&builtin))
}

fn lookup<'a>(
    workspace: &'a Workspace,
    name: &str,
    environment: bool,
    scope: &BTreeSet<Url>,
    aliases: Option<&OnceCell<CommandSignatures<'a>>>,
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
        let mut signature = source_definition(&doc.text.source, &doc.syntax, entry, environment)?;
        if !environment
            && signature.parameters.is_empty()
            && source_alias(&doc.text.source, &doc.syntax, entry).is_some()
        {
            let local = OnceCell::new();
            let signatures = aliases.unwrap_or(&local).get_or_init(|| {
                CommandSignatures::new(
                    workspace
                        .documents
                        .iter()
                        .filter(|(uri, _)| scope.contains(*uri))
                        .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
                )
            });
            signature = signatures.local.get(name)?.as_ref()?.clone();
            signature.name = name.into();
        }
        describe_definition(uri, doc, entry, environment, signature)
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
        } else if matches!(
            name,
            "thefield"
                | "iffieldundef"
                | "iflistundef"
                | "ifnameundef"
                | "iffieldequalstr"
                | "iffieldequals"
                | "iffieldsequal"
                | "iflistequals"
                | "ifnameequals"
                | "ifentrytype"
        ) && workspace.documents.iter().any(|(uri, doc)| {
            scope.contains(uri) && crate::generated::generated_command_exists(doc, name)
        }) {
            // These data accessors can be replaced implicitly by counters,
            // theorems or newif; their builtin argument lists no longer apply.
            None
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
            Kind::RequiredGroup | Kind::CitationKeys(_)
                if source.as_bytes().get(i) != Some(&b'{') =>
            {
                return None
            }
            Kind::Required | Kind::RequiredGroup | Kind::CitationKeys(_) | Kind::CommandToken => {
                (b'{', b'}')
            }
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

fn citation_parameters(style: crate::citations::Style, groups: usize) -> Vec<Parameter> {
    if style.single_key() {
        let labels: &[&str] = if style.context_codes() == 2 {
            &["{key}", "{true}", "{false}"]
        } else {
            &["{key}", "{code}"]
        };
        return labels
            .iter()
            .enumerate()
            .map(|(index, label)| Parameter {
                obey_spaces: false,
                kind: if index == 0 {
                    Kind::CitationKeys(style)
                } else {
                    Kind::Required
                },
                label: (*label).into(),
            })
            .collect();
    }
    if style.category_membership() || style.set_definition() {
        return vec![
            Parameter {
                obey_spaces: false,
                kind: Kind::Required,
                label: if style.set_definition() {
                    "{set key}"
                } else {
                    "{category}"
                }
                .into(),
            },
            Parameter {
                obey_spaces: false,
                kind: Kind::CitationKeys(style),
                label: "{keys}".into(),
            },
        ];
    }
    let mut parameters = Vec::new();
    if style.repeated() {
        parameters.extend(
            ["note / prenote", "postnote"]
                .into_iter()
                .map(|label| Parameter {
                    obey_spaces: false,
                    kind: Kind::Delimited {
                        open: "(".into(),
                        close: ")".into(),
                        optional: true,
                    },
                    label: format!("({label})"),
                }),
        );
    }
    for group in 1..=groups {
        let suffix = if style.repeated() {
            format!(" {group}")
        } else {
            String::new()
        };
        let fields = if style.volume() {
            vec![
                ("prenote", Kind::Optional),
                ("volume", Kind::Required),
                ("pages", Kind::Optional),
            ]
        } else {
            vec![
                ("note / prenote", Kind::Optional),
                ("postnote", Kind::Optional),
            ]
        };
        for (label, kind) in fields {
            let label = if matches!(kind, Kind::Optional) {
                format!("[{label}{suffix}]")
            } else {
                format!("{{{label}{suffix}}}")
            };
            parameters.push(Parameter {
                obey_spaces: false,
                kind,
                label,
            });
        }
        parameters.push(Parameter {
            obey_spaces: false,
            kind: Kind::CitationKeys(style),
            label: format!("{{keys{suffix}}}"),
        });
        if let Some(label) = style.selector() {
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::Optional,
                label: "[format]".into(),
            });
            parameters.push(Parameter {
                obey_spaces: false,
                kind: Kind::Required,
                label: format!("{{{label}}}"),
            });
        }
    }
    parameters
}

fn citation_parameter(
    style: crate::citations::Style,
    group: usize,
    role: crate::citations::Role,
) -> usize {
    use crate::citations::Role;
    if style.single_key() {
        return match role {
            Role::Keys => 0,
            Role::Code(index) => index + 1,
            _ => unreachable!(),
        };
    }
    if style.category_membership() || style.set_definition() {
        return match role {
            Role::Category | Role::SetKey => 0,
            Role::Keys => 1,
            _ => unreachable!(),
        };
    }
    if let Role::Overall(index) = role {
        return index;
    }
    let base =
        if style.repeated() { 2 } else { 0 } + (group - 1) * if style.volume() { 4 } else { 3 };
    base + match role {
        Role::Note(index) => index,
        Role::Volume => 1,
        Role::Pages => 2,
        Role::Keys => {
            if style.volume() {
                3
            } else {
                2
            }
        }
        Role::Format => 3,
        Role::Selector => 4,
        Role::Overall(_) | Role::Category | Role::SetKey | Role::Code(_) => unreachable!(),
    }
}

fn citation_active(
    source: &str,
    end: usize,
    cursor: usize,
    style: crate::citations::Style,
    signature: &mut Signature,
) -> Option<usize> {
    let parsed = crate::citations::arguments(source, end, style);
    let groups = parsed.fields.last().map_or(1, |field| field.group.max(1));
    signature.parameters = citation_parameters(style, groups);
    for field in &parsed.fields {
        if cursor < field.argument.outer_span.end
            || (!field.complete && cursor == field.argument.outer_span.end)
        {
            return Some(citation_parameter(style, field.group, field.role));
        }
    }
    parsed
        .waiting
        .filter(|waiting| waiting.span.start <= cursor && cursor <= waiting.span.end)
        .map(|waiting| citation_parameter(style, waiting.group, waiting.role))
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
            && !doc.syntax.literal_tests.contains(span)
            && !doc.syntax.commands.iter().any(|command| {
                (matches!(command.name.as_str(), "url" | "href" | "hyperref")
                    || crate::minted::is_inline(&command.name))
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
        .filter(|(_, command)| {
            command.span.end <= cursor && !doc.syntax.control_operands.contains(&command.span.start)
        })
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
        if doc
            .syntax
            .contextual_command_shapes
            .get(&command.span.start)
            .is_some_and(|shape| shape.takes_no_arguments())
        {
            continue;
        }
        let signature = if let Some(arity) = doc.syntax.string_test_arities.get(&command.span.start)
        {
            let Some(count) = arity else {
                continue;
            };
            let mut signature =
                operand_builtin(if *count == 2 { "ifstrequal" } else { "ifblank" })?;
            signature.name = name.into();
            Some(signature)
        } else if name == "hyperref"
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
            })
        {
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
            .and_then(|argument| lookup(workspace, &argument.value, true, &scope, None))
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
            if let Some(style) =
                signature
                    .parameters
                    .iter()
                    .find_map(|parameter| match parameter.kind {
                        Kind::CitationKeys(style) => Some(style),
                        _ => None,
                    })
            {
                if let Some(active) =
                    citation_active(&doc.text.source, end, cursor, style, &mut signature)
                {
                    return Some(signature.information(active));
                }
                continue;
            }
            if let Some(active) = active_parameter(doc, command, end, cursor, &signature) {
                return Some(signature.information(active));
            }
        }
    }
    None
}

#[cfg(test)]
mod tex_operand_tests {
    use super::*;
    use crate::text::Text;

    #[test]
    fn copied_literal_candidates_capture_builtin_and_previous_source_shapes() {
        for (source, expected) in [
            (
                r"\def\original#1#2{}\let\bad\original\def\original{}\bad x&",
                true,
            ),
            (
                r"\newenvironment{bad}[2]{}{}\let\start\begin\def\begin{}\let\begin\start\start{bad}x&",
                true,
            ),
            (
                r"\def\consume#1#2{}\def\bad{}\def\let{}\let\bad\consume x&",
                true,
            ),
            (r"\let\first\second\let\second\first\first x&", false),
            (r"\def\original{}\let\bad\original\bad{x}{y}&", false),
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            assert_eq!(
                signatures.literal_is_argument(source, &parsed, source.rfind('&').unwrap()),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn copied_sources_are_operands_and_do_not_execute_file_loads() {
        for copy in [
            r"\let\bad\input",
            r"\let\bad=\input",
            r"\NewCommandCopy{\bad}{\input}",
            r"\cslet{bad}\input",
            r"\letcs\bad{input}",
            r"\csletcs{bad}{input}",
        ] {
            let source = format!(r"{copy}{{wrong}}\input{{right}}");
            let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
            let signatures = CommandSignatures::source_context([(source.as_str(), &parsed)]);
            let active = signatures.source_command_positions(&source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
            {
                assert_eq!(
                    active.contains(&command.span.start),
                    command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.value == "right"),
                    "{source}: {command:?}"
                );
            }
        }
    }

    #[test]
    fn copy_sources_require_one_static_operand_and_keep_primitive_braces_literal() {
        for source in [
            r"\let\bad{\source}",
            r"\let\bad}\source",
            r"\NewCommandCopy{\bad}{\source\other}",
            r"\csletcs{bad}{\dynamic}",
            r"\letcs\bad{source\dynamic}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let entry = parsed
                .definitions
                .iter()
                .find(|entry| entry.name == "bad")
                .unwrap();
            assert!(
                literal_copy_target(source, &parsed, entry, false).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn paused_literal_candidates_preserve_complete_arguments_without_certifying_meanings() {
        for (declaration, call, consumed) in [
            (
                r"\newenvironment{bad}[2][default]{}{}",
                r"\begin{bad}{x}&",
                false,
            ),
            (
                r"\newenvironment{bad}[2][default]{}{}",
                r"\begin{bad}&",
                true,
            ),
            (r"\def\bad#1#2{}", r"\bad{x}{y}&", false),
            (r"\def\bad#1#2{}", r"\bad{x}&", true),
            (r"\def\bad#2{}", r"\bad{x}&", true),
            (r"\def\bad{\undefined}", r"\bad{x}&", true),
            (r"\def\bad{\halign}", r"\bad{#&#\cr A&B&C\cr}", false),
        ] {
            let definitions = syntax::Syntax::latex(&Text::new(declaration.into()));
            let parsed = syntax::Syntax::latex(&Text::new(call.into()));
            let signatures = CommandSignatures::with_ambiguous_names([(call, &parsed)], ["bad"])
                .with_literal_sources([(declaration, &definitions)]);
            assert!(signatures.local["bad"].is_none());
            assert_eq!(
                signatures.literal_is_argument(call, &parsed, call.rfind('&').unwrap()),
                consumed,
                "{declaration}/{call}"
            );
        }
    }

    #[test]
    fn possible_literal_arguments_follow_long_aliases_and_terminate_cycles() {
        let name = |index: usize| format!("alias{}", "a".repeat(index + 1));
        let mut source = String::from(r"\def\bad{}\def\bad#1#2{&}");
        for index in 0..512 {
            source.push_str(&format!("\\def\\{}{{\\{}}}", name(index), name(index + 1)));
        }
        source.push_str(&format!("\\def\\{}{{\\bad}}\\{} x&", name(512), name(0)));
        let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
        let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
        assert!(signatures.local["bad"].is_none());
        assert!(signatures.literal_is_argument(&source, &parsed, source.len() - 1));
        for (definitions, expected) in [
            (r"\def\first{\second}\def\second{\first}", false),
            (
                r"\def\first{\second}\def\second{\first}\def\second{\bad}\def\bad#1#2{&}",
                true,
            ),
        ] {
            let source = format!("{definitions}\\first x&");
            let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
            let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
            assert_eq!(
                signatures.literal_is_argument(&source, &parsed, source.len() - 1),
                expected
            );
        }
    }

    #[test]
    fn possible_literal_arguments_preserve_completed_calls_and_deferred_declarations() {
        for source in [
            r"\def\bad{}\def\bad{}\bad x&",
            r"\def\bad#1#2{}\def\bad#1#2{}\bad xy&",
            r"\def\bad#1;{}\def\bad#1;{}\bad x;&",
            r"\def\bad#1&#2{}\def\provider{\bad x}&",
            r"\def\bad#1&#2{}\def\provider#1\bad#2{}x&",
            r"\def\bad#1#2{}\def\bad#1#2{}\def\show{\textbf}\show{x}&",
            r"\newenvironment{bad}[2]{}{}\renewenvironment{bad}[2]{}{}\begin{bad}xy&\end{bad}",
        ] {
            let at = source.rfind('&').unwrap();
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            assert!(
                !signatures.possible_literal_argument(source, &parsed, at),
                "{source}"
            );
            assert!(
                !signatures.literal_is_argument(source, &parsed, at),
                "{source}"
            );
        }
    }

    #[test]
    fn graphicspath_consumes_only_its_data_argument_and_preserves_following_calls() {
        for call in [
            r"\graphicspath",
            r"\csuse{graphicspath}",
            r"\csname graphicspath\endcsname",
            r"\setpaths",
            r"\csuse{setpaths}",
            r"\csname setpaths\endcsname",
        ] {
            for argument in [r"{\input{opaque}}", r"\input{opaque}"] {
                let source = format!(
                    r"\def\setpaths{{\graphicspath}}{call}{argument}\includegraphics{{picture}}\input{{right}}"
                );
                let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                let signatures = CommandSignatures::source_context([(source.as_str(), &parsed)]);
                let operands = signatures.argument_tokens(&source, &parsed);
                for command in parsed
                    .commands
                    .iter()
                    .filter(|command| matches!(command.name.as_str(), "input" | "includegraphics"))
                {
                    assert_eq!(
                        operands.contains(&command.span.start),
                        command
                            .argument
                            .as_ref()
                            .is_some_and(|argument| argument.value == "opaque"),
                        "{source}: {command:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn bare_environment_names_dispatch_the_actual_environment_parameters() {
        for call in [
            r"\begin x",
            r"\csuse{begin}x",
            r"\csname begin\endcsname x",
            r"\start x",
            r"\csuse{start}x",
            r"\csname start\endcsname x",
            "\\begin% name\r\nx",
        ] {
            for (definition, prefix, tail) in [
                (r"\newenvironment{x}[3]{}{}", r"\relax", ""),
                (r"\newenvironment{x}[4][default]{}{}", r"[note]\relax", ""),
                (
                    r"\NewDocumentEnvironment{x}{O{default} m m m}{}{}",
                    r"[note]\relax",
                    "",
                ),
                (r"\newenvironment{x}[1]{}{}", "{", "}"),
                (r"\NewDocumentEnvironment{x}{d()}{}{}", "(", ")"),
                (r"\NewDocumentEnvironment{x}{t\mark m}{}{}", r"\mark", ""),
            ] {
                let source = format!(
                    r"{definition}\def\start{{\begin}}{call}{prefix}\input{{wrong}}{tail}\input{{right}}"
                );
                let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
                let operands = signatures.argument_tokens(&source, &parsed);
                for command in parsed
                    .commands
                    .iter()
                    .filter(|command| command.name == "input")
                {
                    assert_eq!(
                        operands.contains(&command.span.start),
                        command.argument.as_ref().unwrap().value == "wrong",
                        "{source}"
                    );
                }
            }
        }
    }

    #[test]
    fn bare_environment_names_require_an_engine_independent_literal_token() {
        for value in ["x", "7", "-", "[", "]"] {
            let source = format!("% name\r\n{value}tail");
            let argument = syntax::environment_name_argument(&source, 0).unwrap();
            assert_eq!(argument.value, value);
            assert_eq!(argument.span, argument.outer_span);
            assert_eq!(&source[argument.span], value);
        }
        for value in [r"\which", "é", "😀", "^^78", "{x", "}", "#1", "", "% end"] {
            assert!(
                syntax::environment_name_argument(value, 0).is_none(),
                "{value}"
            );
        }
        let source = "{% name\r\nx% keep\r\n}";
        let argument = syntax::environment_name_argument(source, 0).unwrap();
        assert_eq!(argument.value, "x");
        assert_eq!(&source[argument.span.clone()], "x");
        assert_ne!(argument.span, argument.outer_span);
    }

    #[test]
    fn environment_arguments_are_operands_through_begin_direct_and_named_calls() {
        for call in [
            r"\begin{wrapped}",
            r"\csuse{begin}{wrapped}",
            r"\csname begin\endcsname{wrapped}",
            r"\start{wrapped}",
            r"\csuse{start}{wrapped}",
            r"\csname start\endcsname{wrapped}",
            "\\csuse{begin}{% name\r\nwrapped% keep\r\n}",
            r"\wrapped",
            r"\csuse{wrapped}",
            r"\csname wrapped\endcsname",
        ] {
            for (definition, prefix, tail) in [
                (r"\newenvironment{wrapped}[3]{}{}", r"\relax", ""),
                (
                    r"\newenvironment{wrapped}[4][default]{}{}",
                    r"[note]\relax",
                    "",
                ),
                (
                    r"\NewDocumentEnvironment{wrapped}{O{default} m m m}{}{}",
                    r"[note]\relax",
                    "",
                ),
                (r"\newenvironment{wrapped}[1]{}{}", "{", "}"),
                (r"\NewDocumentEnvironment{wrapped}{d()}{}{}", "(", ")"),
                (
                    r"\NewDocumentEnvironment{wrapped}{t\mark m}{}{}",
                    r"\mark",
                    "",
                ),
            ] {
                let source = format!(
                    r"{definition}\def\start{{\begin}}{call}{prefix}\input{{wrong}}{tail}\input{{right}}"
                );
                let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
                let operands = signatures.argument_tokens(&source, &parsed);
                for command in parsed
                    .commands
                    .iter()
                    .filter(|command| command.name == "input")
                {
                    assert_eq!(
                        operands.contains(&command.span.start),
                        command.argument.as_ref().unwrap().value == "wrong",
                        "{source}"
                    );
                }
            }
        }
    }

    #[test]
    fn indirect_begins_preserve_completed_arguments_and_builtin_environment_bodies() {
        for begin in [
            r"\begin",
            r"\csuse{begin}",
            r"\csname begin\endcsname",
            r"\start",
            r"\csuse{start}",
            r"\csname start\endcsname",
        ] {
            for (declaration, call) in [
                (r"\newenvironment{wrapped}{}{}", "{wrapped}"),
                (r"\newenvironment{wrapped}[1]{}{}", "{wrapped}{Unused}"),
                (r"\newenvironment{wrapped}[1][default]{}{}", "{wrapped}"),
                (
                    r"\NewDocumentEnvironment{wrapped}{d()}{}{}",
                    "{wrapped}(note)",
                ),
                (r"\NewDocumentEnvironment{wrapped}{!s}{}{}", "{wrapped} "),
                ("", "{document}"),
                ("", "{minipage}{3cm}"),
                ("", "{figure}[ht]"),
                // A replacement begin consumes only its own arguments.
                (
                    r"\newenvironment{wrapped}[3]{}{}\renewcommand{\begin}[1]{}",
                    "{wrapped}",
                ),
            ] {
                let source =
                    format!(r"{declaration}\def\start{{\begin}}{begin}{call}\input{{right}}");
                let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
                let operands = signatures.argument_tokens(&source, &parsed);
                for command in parsed
                    .commands
                    .iter()
                    .filter(|command| command.name == "input")
                {
                    assert!(!operands.contains(&command.span.start), "{source}");
                }
            }
        }
        let source = r"\newenvironment{wrapped}[3]{}{}\def\start{\begin}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        for ambiguous in ["begin", "start"] {
            let signatures =
                CommandSignatures::with_ambiguous_names([(source, &parsed)], [ambiguous]);
            assert!(signatures.local["start"].is_none(), "{ambiguous}");
        }
    }

    #[test]
    fn completed_environment_arguments_preserve_the_body_and_following_commands() {
        for source in [
            r"\newenvironment{wrapped}{}{}\begin{wrapped}\input{right}\end{wrapped}",
            r"\newenvironment{wrapped}[0]{}{}\begin{wrapped}\input{right}\end{wrapped}",
            r"\newenvironment{wrapped}[1]{}{}\begin{wrapped}{Unused}\input{right}\end{wrapped}",
            r"\newenvironment{wrapped}[1][default]{}{}\begin{wrapped}\input{right}\end{wrapped}",
            r"\NewDocumentEnvironment{wrapped}{d()}{}{}\begin{wrapped}(Unused)\input{right}\end{wrapped}",
            r"\newenvironment{wrapped}{}{}\def\bridge{\wrapped}\bridge\input{right}",
            r"\begin{document}{\input{right}}\end{document}",
            r"\begin{minipage}{3cm}\input{right}\end{minipage}",
            r"\begin{figure}[ht]\input{right}\end{figure}",
            r"\newcommand{\begin}[1]{}\begin{wrapped}\input{right}",
            r"\newenvironment{begin}{}{}\input{right}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            let operands = signatures.argument_tokens(source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
            {
                assert!(!operands.contains(&command.span.start), "{source}");
            }
        }
    }

    #[test]
    fn generated_environment_end_controls_and_aliases_keep_real_following_commands_visible() {
        for source in [
            r"\newenvironment{wrapped}{}{}\wrapped\endwrapped\input{right}",
            r"\newenvironment{wrapped}{}{}\csuse{wrapped}\csuse{endwrapped}\input{right}",
            r"\newenvironment{wrapped}{}{}\csname wrapped\endcsname\csname endwrapped\endcsname\input{right}",
            r"\newenvironment{wrapped}{}{}\def\finish{\endwrapped}\wrapped\finish\input{right}",
            r"\newenvironment{wrapped}{\consume}{}\newcommand{\consume}[1]{}\wrapped\input{wrong}\input{right}",
            r"\newenvironment{wrapped}{}{\consume}\newcommand{\consume}[1]{}\endwrapped\input{wrong}\input{right}",
            r"\NewDocumentEnvironment{wrapped}{}{\consume}{}\newcommand{\consume}[1]{}\wrapped\input{wrong}\input{right}",
            r"\NewDocumentEnvironment{wrapped}{}{}{\consume}\newcommand{\consume}[1]{}\endwrapped\input{wrong}\input{right}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            let operands = signatures.argument_tokens(source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
            {
                assert_eq!(
                    operands.contains(&command.span.start),
                    command.argument.as_ref().unwrap().value == "wrong",
                    "{source}"
                );
            }
        }
        let source = r"\newenvironment{wrapped}{}{}\def\finish{\endwrapped}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        let paused =
            CommandSignatures::with_ambiguous_environments([(source, &parsed)], [], ["wrapped"]);
        assert!(paused.local["wrapped"].is_none());
        assert!(paused.local["endwrapped"].is_none());
        assert!(paused.local["finish"].is_none());
        for source in [
            r"\newenvironment{wrapped}{}{}\renewcommand{\endwrapped}[1]{}\def\finish{\endwrapped}",
            r"\newenvironment{wrapped}{\endwrapped}{\wrapped}\def\finish{\endwrapped}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            assert!(signatures.local["endwrapped"].is_none(), "{source}");
            assert!(signatures.local["finish"].is_none(), "{source}");
        }
    }

    #[test]
    fn environment_declaration_metadata_is_data_while_start_and_end_code_stays_visible() {
        for source in [
            r"\newenvironment{wrapped}[1][\input{wrong}]{\input{right}}{\input{right}}",
            r"\NewDocumentEnvironment{wrapped}{O{\input{wrong}}}{\input{right}}{\input{right}}",
            r"\NewEnviron{wrapped}[1][\input{wrong}]{\input{right}}[\input{right}]",
            r"\def\declare{\NewDocumentEnvironment}\declare{wrapped}{O{\input{wrong}}}{\input{right}}{\input{right}}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            let operands = signatures.argument_tokens(source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
            {
                assert_eq!(
                    operands.contains(&command.span.start),
                    command.argument.as_ref().unwrap().value == "wrong",
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn environment_start_signatures_preserve_namespace_conflicts_and_paused_aliases() {
        let source = r"\newenvironment{wrapped}[3]{}{}\def\bridge{\wrapped}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        let signatures = CommandSignatures::new([(source, &parsed)]);
        assert_eq!(
            signatures.local["wrapped"]
                .as_ref()
                .unwrap()
                .parameters
                .len(),
            3
        );
        assert_eq!(
            signatures.local["bridge"]
                .as_ref()
                .unwrap()
                .parameters
                .len(),
            3
        );
        let paused = CommandSignatures::with_ambiguous_names([(source, &parsed)], ["wrapped"]);
        assert!(paused.local["wrapped"].is_none());
        assert!(paused.local["bridge"].is_none());
        for source in [
            r"\newenvironment{wrapped}[3]{}{}\renewcommand{\wrapped}[1]{}\def\bridge{\wrapped}",
            r"\newenvironment{wrapped}[3]{}{}\renewenvironment{wrapped}[1]{}{}\def\bridge{\wrapped}",
            r"\NewDocumentEnvironment{wrapped}{b}{}{}\def\bridge{\wrapped}",
            r"\NewEnvironmentCopy{wrapped}{other}\def\bridge{\wrapped}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            assert!(signatures.local["wrapped"].is_none(), "{source}");
            assert!(signatures.local["bridge"].is_none(), "{source}");
        }
    }

    #[test]
    fn named_macro_invocations_preserve_operand_boundaries_and_following_commands() {
        for named in [r"\csuse{wrapped}", r"\csname wrapped\endcsname"] {
            for (definition, prefix, tail) in [
                (r"\newcommand{\wrapped}[3]{}", r"\relax", ""),
                (r"\newcommand{\wrapped}[4][default]{}", r"[note]\relax", ""),
                (r"\def\wrapped#1#2#3{}", r"\relax", ""),
                (r"\def\wrapped#1#2#3\stop{}", r"\relax", r"\stop"),
                (r"\def\wrapped#1 #2#3#4\stop{}", r" Unused \relax", r"\stop"),
                (
                    r"\NewDocumentCommand{\wrapped}{s t+ O{default} m m m}{}",
                    r"*+[note]\relax",
                    "",
                ),
                (r"\newcommand{\wrapped}[1]{}", "{", "}"),
                (r"\NewDocumentCommand{\wrapped}{d()}{}", "(", ")"),
            ] {
                let source =
                    format!(r"{definition}{named}{prefix}\input{{wrong}}{tail}\input{{right}}");
                let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
                let operands = signatures.argument_tokens(&source, &parsed);
                let calls: Vec<_> = parsed
                    .commands
                    .iter()
                    .filter(|command| command.name == "input")
                    .collect();
                assert_eq!(calls.len(), 2);
                assert!(operands.contains(&calls[0].span.start), "{source}");
                assert!(!operands.contains(&calls[1].span.start), "{source}");
            }
        }
    }

    #[test]
    fn named_invocations_keep_name_group_spaces_and_literal_definitions_distinct() {
        for source in [
            r"\csdef{wrapped} #1\stop{}\csuse{wrapped} \input{wrong}\stop\input{right}",
            r"\expandafter\def\csname wrapped\endcsname#1{\input{right}}",
            r"\newcommand{\wrapped}[1]{}\renewcommand{\csuse}[1]{}\csuse{wrapped}\input{right}",
            r"\newcommand{\wrapped}[1]{}\newcommand{\discard}[1]{}\discard\csname wrapped\endcsname\relax\input{right}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::new([(source, &parsed)]);
            let operands = signatures.argument_tokens(source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
            {
                assert_eq!(
                    operands.contains(&command.span.start),
                    command.argument.as_ref().unwrap().value == "wrong",
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn tex_delimiters_skip_groups_match_whole_tokens_and_preserve_completed_calls() {
        for (definition, call) in [
            (
                r"\def\wrapped#1 #2#3#4\stop{}",
                r"\wrapped Unused \relax\input{wrong}\stop",
            ),
            (
                "\\def\\wrapped#1 \t  #2#3#4\\stop{}",
                "\\wrapped Unused\t\\relax\\input{wrong}\\stop",
            ),
            (
                "\\def\\wrapped#1\r\n #2#3#4\\stop{}",
                "\\wrapped Unused\r\n \\relax\\input{wrong}\\stop",
            ),
            (
                "\\def\\wrapped#1 % keep\r\n #2#3#4\\stop{}",
                "\\wrapped Unused % keep\r\n \\relax\\input{wrong}\\stop",
            ),
            (
                r"\def\wrapped#1a #2\stop{}",
                r"\wrapped\input{wrong}a \relax\stop",
            ),
            (
                r"\def\wrapped#1\marker   #2\stop{}",
                r"\wrapped\input{wrong}\marker payload\stop",
            ),
            (
                r"\def\wrapped#1\% #2\stop{}",
                r"\wrapped\input{wrong}\% payload\stop",
            ),
            (
                r"\def\wrapped#1\    #2\stop{}",
                r"\wrapped\input{wrong}\ payload\stop",
            ),
            (
                r"\def\wrapped#1#2#3\stop{}",
                r"\wrapped\relax\input{wrong}\stop",
            ),
            (
                r"\def\wrapped#1\stop{}",
                r"\wrapped\relax\input{wrong}\stop",
            ),
            (
                r"\def\wrapped#1\stop{}",
                r"\wrapped{\stop\input{wrong}}\stopper\input{wrong}\stop",
            ),
            (
                r"\def\wrapped\start#1\mid#2\stop{}",
                r"\wrapped\start\relax\mid\input{wrong}\stop",
            ),
            (r"\def\wrapped#1aba{}", r"\wrapped ab\input{wrong}aaba"),
            (r"\def\wrapped#1;!{}", r"\wrapped; !\input{wrong};!"),
            (
                "\\def\\wrapped#1a% keep\r\n b{}",
                "\\wrapped a% keep\r\n \\input{wrong}ab",
            ),
            (
                r"\def\wrapped#1\stop{}\def\bridge{\wrapped}",
                r"\bridge\input{wrong}\stop",
            ),
        ] {
            let source = format!("{definition}\n{call}\\input{{right}}");
            let text = Text::new(source.clone());
            let parsed = syntax::Syntax::latex(&text);
            let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
            let operands = signatures.argument_tokens(&source, &parsed);
            let calls: Vec<_> = parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
                .collect();
            assert!(calls.len() >= 2, "{source}");
            for command in &calls[..calls.len() - 1] {
                assert!(operands.contains(&command.span.start), "{source}");
            }
            assert!(
                !operands.contains(&calls.last().unwrap().span.start),
                "{source}"
            );
        }
    }

    #[test]
    fn tex_operand_aliases_preserve_ambiguity_and_do_not_add_signature_help() {
        let source = r"\def\wrapped#1\stop{}\def\bridge{\wrapped}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        let signatures = CommandSignatures::new([(source, &parsed)]);
        assert!(signatures.local["wrapped"].is_none());
        assert!(signatures.local["bridge"].is_none());
        assert!(signatures.tex_operands["wrapped"].is_some());
        assert!(signatures.tex_operands["bridge"].is_some());
        let paused = CommandSignatures::with_ambiguous_names([(source, &parsed)], ["wrapped"]);
        assert!(paused.tex_operands["wrapped"].is_none());
        assert!(paused.tex_operands["bridge"].is_none());
        let duplicate = format!("{source}\\def\\wrapped#1#2\\stop{{}}");
        let parsed = syntax::Syntax::latex(&Text::new(duplicate.clone()));
        let signatures = CommandSignatures::new([(duplicate.as_str(), &parsed)]);
        assert!(signatures.tex_operands["wrapped"].is_none());
        assert!(signatures.tex_operands["bridge"].is_none());
    }

    #[test]
    fn malformed_and_catcode_sensitive_tex_parameter_text_is_not_inferred() {
        for source in [
            r"\def\wrapped#2\stop{}",
            r"\def\wrapped#1#1\stop{}",
            "\\def\\wrapped#1\n\n#2\\stop{}",
            "\\def\\wrapped\n\n#1{}",
            r"\def\wrapped#1^^41{}",
            r"\def\wrapped#1é{}",
            r"\def\wrapped#1#{}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            assert!(!parsed.definitions.is_empty(), "{source}");
            assert!(
                TexOperands::definition(source, &parsed, &parsed.definitions[0]).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn tex_space_tokens_do_not_confuse_named_prefixes_comments_and_paragraphs() {
        for (source, prefix, delimiters) in [
            (
                r"\def\wrapped#1   #2\stop{}",
                vec![],
                vec![vec![" "], vec![r"\stop"]],
            ),
            (
                r"\csdef{wrapped} #1\stop{}",
                vec![" "],
                vec![vec![r"\stop"]],
            ),
            (
                "\\def\\wrapped#1% join\r\n #2{}",
                vec![],
                vec![vec![], vec![]],
            ),
            (
                "\\def\\wrapped#% join\r\n 1 #2{}",
                vec![],
                vec![vec![" "], vec![]],
            ),
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let (operands, _) =
                TexOperands::definition(source, &parsed, &parsed.definitions[0]).unwrap();
            assert_eq!(operands.prefix, prefix, "{source}");
            assert_eq!(operands.delimiters, delimiters, "{source}");
        }
        let source = r"\def\wrapped#1 #2\stop{}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        let (operands, _) =
            TexOperands::definition(source, &parsed, &parsed.definitions[0]).unwrap();
        for call in [
            " Unused\n\npayload\\stop",
            " Unused % keep\r\n \r\npayload\\stop",
        ] {
            assert!(operands.end(call, 0, true).is_none(), "{call}");
        }
        let source = r"\csdef{wrapped} #1\stop{}";
        let parsed = syntax::Syntax::latex(&Text::new(source.into()));
        let (operands, _) =
            TexOperands::definition(source, &parsed, &parsed.definitions[0]).unwrap();
        assert!(operands.end(r" Unused\stop", 0, true).is_none());
    }

    #[test]
    fn tex_definition_forms_share_bounded_operand_rules() {
        for declaration in [
            "def", "gdef", "edef", "xdef", "csdef", "csgdef", "csedef", "csxdef",
        ] {
            let target = if declaration.starts_with("cs") {
                "{wrapped}"
            } else {
                r"\wrapped"
            };
            let source = format!(
                r"\{declaration}{target}#1#2\stop{{}}\wrapped\relax\input{{wrong}}\stop\input{{right}}"
            );
            let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
            let signatures = CommandSignatures::new([(source.as_str(), &parsed)]);
            assert!(
                signatures.tex_operands["wrapped"].is_some(),
                "{declaration}"
            );
            let operands = signatures.argument_tokens(&source, &parsed);
            let calls: Vec<_> = parsed
                .commands
                .iter()
                .filter(|command| command.name == "input")
                .collect();
            assert_eq!(calls.len(), 2);
            assert!(operands.contains(&calls[0].span.start), "{declaration}");
            assert!(!operands.contains(&calls[1].span.start), "{declaration}");
        }
        for count in [64, 65] {
            let source = format!(r"\def\wrapped#1{}{{}}", "a".repeat(count));
            let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
            assert_eq!(
                TexOperands::definition(&source, &parsed, &parsed.definitions[0]).is_some(),
                count == 64
            );
        }
    }
}

#[cfg(test)]
mod code_configuration_operand_tests {
    use super::*;
    use crate::text::Text;

    #[test]
    fn code_settings_stop_at_one_data_argument_before_independent_file_calls() {
        for name in ["lstset", "fvset"] {
            for call in [
                format!("\\{name}"),
                format!("\\csuse{{{name}}}"),
                format!("\\csname {name}\\endcsname"),
                r"\settings".into(),
                r"\csuse{settings}".into(),
                r"\csname settings\endcsname".into(),
            ] {
                for argument in [r"{\input{opaque}}", r"\input{opaque}"] {
                    let source = format!(
                        r"\def\settings{{\{name}}}{call}{argument}\verbatiminput{{code.txt}}\lstinputlisting{{code.txt}}\VerbatimInput{{code.txt}}\input{{right}}"
                    );
                    let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
                    let signatures =
                        CommandSignatures::source_context([(source.as_str(), &parsed)]);
                    let operands = signatures.argument_tokens(&source, &parsed);
                    for command in parsed.commands.iter().filter(|command| {
                        syntax::code_input_command(&command.name) || command.name == "input"
                    }) {
                        assert_eq!(
                            operands.contains(&command.span.start),
                            command
                                .argument
                                .as_ref()
                                .is_some_and(|argument| argument.value == "opaque"),
                            "{source}: {command:?}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod chapter_selector_operand_boundaries {
    use super::*;
    use crate::text::Text;

    #[test]
    fn chapter_selection_ends_before_following_inputs_and_keeps_its_names_as_data() {
        for call in [
            r"\includeonly",
            r"\csuse{includeonly}",
            r"\csname includeonly\endcsname",
            r"\choose",
            r"\csuse{choose}",
            r"\csname choose\endcsname",
        ] {
            let source = format!(
                r"\def\choose{{\includeonly}}{call}{{\input{{opaque}}}}\input{{settings}}\include{{chapter}}"
            );
            let parsed = syntax::Syntax::latex(&Text::new(source.clone()));
            let signatures = CommandSignatures::source_context([(source.as_str(), &parsed)]);
            let operands = signatures.argument_tokens(&source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| matches!(command.name.as_str(), "input" | "include"))
            {
                assert_eq!(
                    operands.contains(&command.span.start),
                    command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.value == "opaque"),
                    "{source}: {command:?}"
                );
            }
        }
    }
}
#[cfg(test)]
mod quoted_source_token_tests {
    use super::*;
    use crate::text::Text;

    #[test]
    fn meaning_quotes_one_control_token_without_consuming_groups_or_forwarded_operands() {
        for source in [
            r"\meaning\input{quoted}\input{real}",
            "\\meaning% before 😀\r\n\\input{quoted}\\input{real}",
            r"\meaning{\input{real}}\input{real}",
            r"\meaning*\input{real}",
            r"\def\meaning#1{#1}\meaning\input{real}",
            r"\newcommand{\execute}[1]{#1}\execute{\meaning\input{real}}",
            r"\csuse{meaning}\input{quoted}\input{real}",
            r"\csname meaning\endcsname\input{quoted}\input{real}",
            r"\meaning\include{quoted}\include{real}",
        ] {
            let parsed = syntax::Syntax::latex(&Text::new(source.into()));
            let signatures = CommandSignatures::source_context([(source, &parsed)]);
            let quoted = signatures.quoted_command_tokens(source, &parsed);
            for command in parsed
                .commands
                .iter()
                .filter(|command| matches!(command.name.as_str(), "input" | "include"))
            {
                assert_eq!(
                    quoted.contains(&command.span.start),
                    // Fixture names identify expected operands independently
                    // of whether Syntax gives the control call arguments.
                    source[command.span.end..].starts_with("{quoted}"),
                    "{source}: {command:?}"
                );
            }
        }
    }
}
