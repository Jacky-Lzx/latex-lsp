//! Static TeX mode tracking for command completion; no macro expansion.
use crate::parser::syntax::{self, Syntax};

/// Track only the source before the control sequence being completed. Ignored
/// text and unexpanded operands must not open formulas in the surrounding text.
pub(super) fn is_math(source: &str, syntax: &Syntax, offset: usize) -> bool {
    let bytes = source.as_bytes();
    let mut ignored: Vec<_> = syntax
        .ignored
        .iter()
        .chain(&syntax.literal_conversions)
        .filter(|span| span.start < offset)
        .collect();
    ignored.sort_unstable_by_key(|span| span.start);
    let mut ignored = ignored.into_iter().peekable();
    let mut at = 0;
    let mut math = false;
    let mut groups = Vec::new();
    let mut environments = Vec::new();
    while at < offset {
        while ignored.peek().is_some_and(|span| span.end <= at) {
            ignored.next();
        }
        if let Some(span) = ignored.peek().filter(|span| span.start <= at) {
            at = span.end;
            continue;
        }
        if syntax.punctuation_operands.contains(&at) {
            at += 1;
            continue;
        }
        match bytes[at] {
            b'\\' => {
                let start = at;
                at += 1;
                let name_start = at;
                while at < bytes.len() && (bytes[at].is_ascii_alphabetic() || bytes[at] == b'@') {
                    at += 1;
                }
                if at == name_start && at < bytes.len() {
                    // Consume a complete control symbol, including escaped
                    // dollars/braces and the second slash in a line break.
                    at += source[at..].chars().next().unwrap().len_utf8();
                }
                if syntax.control_operands.contains(&start) {
                    continue;
                }
                let name = &source[name_start..at];
                match name {
                    "(" | "[" => math = true,
                    ")" | "]" => math = false,
                    "begin" | "end" => {
                        if let Some(arg) = syntax::environment_name_argument(source, at) {
                            if arg.outer_span.end > offset {
                                continue;
                            }
                            at = arg.outer_span.end;
                            if math_environment(&arg.value) {
                                if name == "begin" {
                                    environments.push((arg.value, math));
                                    math = true;
                                } else if let Some(index) =
                                    environments.iter().rposition(|(env, _)| *env == arg.value)
                                {
                                    math = environments[index].1;
                                    environments.truncate(index);
                                }
                            }
                        }
                    }
                    "ensuremath" | "text" | "textrm" | "textnormal" | "textup" | "textbf"
                    | "textmd" | "textit" | "textsl" | "textsc" | "textsf" | "texttt" | "mbox"
                    | "hbox" | "vbox" | "operatorname" => {
                        let mut argument = syntax::skip_space(source, at);
                        if name == "operatorname" && bytes.get(argument) == Some(&b'*') {
                            argument = syntax::skip_space(source, argument + 1);
                        }
                        if argument < offset && bytes.get(argument) == Some(&b'{') {
                            groups.push(math);
                            math = name == "ensuremath";
                            at = argument + 1;
                        }
                    }
                    _ => {}
                }
            }
            b'$' => {
                math = !math;
                at += if bytes.get(at + 1) == Some(&b'$') {
                    2
                } else {
                    1
                };
            }
            b'{' => {
                groups.push(math);
                at += 1;
            }
            b'}' => {
                if let Some(outer) = groups.pop() {
                    math = outer;
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    math
}

fn math_environment(name: &str) -> bool {
    matches!(
        name.strip_suffix('*').unwrap_or(name),
        "math"
            | "displaymath"
            | "equation"
            | "eqnarray"
            | "align"
            | "alignat"
            | "xalignat"
            | "xxalignat"
            | "flalign"
            | "gather"
            | "multline"
    )
}
