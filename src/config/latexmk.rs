//! Read literal project output settings without executing a Perl rc file.
use std::{fs::File, io::Read, path::Path};

const MAX_RC_SIZE: u64 = 64 * 1024;

pub(super) fn output_directory(path: &Path) -> Option<String> {
    crate::diagnostics::diagnostic_inputs::observe(path);
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut source = String::new();
    file.take(MAX_RC_SIZE + 1)
        .read_to_string(&mut source)
        .ok()?;
    if source.len() as u64 > MAX_RC_SIZE {
        return None;
    }
    literal_output_directory(&source)
}

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    String(Option<String>),
    Symbol(char),
}

fn literal_output_directory(source: &str) -> Option<String> {
    let mut chars = source.chars().peekable();
    let mut statement = Vec::new();
    let mut mentions_output = false;
    let mut depth = 0usize;
    let mut output = None;
    while let Some(ch) = chars.next() {
        let token = match ch {
            ch if ch.is_whitespace() => continue,
            '#' => {
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }
            '\'' | '"' | '`' => Token::String(quoted(&mut chars, ch)?),
            // Perl's quote-like operators, regexes, heredocs and POD can
            // contain apparent assignments. Do not guess their contents.
            '/' | '\\' => return None,
            '<' if chars.peek() == Some(&'<') => return None,
            '=' if chars.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) => return None,
            ch if ch.is_ascii_alphabetic() || matches!(ch, '$' | '_') => {
                let mut word = String::from(ch);
                while chars
                    .peek()
                    .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                {
                    word.push(chars.next().unwrap());
                }
                if matches!(word.as_str(), "__DATA__" | "__END__") {
                    break;
                }
                if matches!(
                    word.as_str(),
                    "q" | "qq" | "qw" | "qx" | "qr" | "m" | "s" | "tr" | "y"
                ) {
                    return None;
                }
                if word == "$out_dir" {
                    mentions_output = true;
                    if depth != 0 {
                        // Conditional assignments and subroutines are not
                        // unconditional project settings.
                        output = None;
                    }
                }
                Token::Word(word)
            }
            '{' | '(' | '[' => {
                depth += 1;
                Token::Symbol(ch)
            }
            '}' | ')' | ']' => {
                depth = depth.checked_sub(1)?;
                Token::Symbol(ch)
            }
            ';' if depth == 0 => {
                if mentions_output {
                    output = match statement.as_slice() {
                        [Token::Word(name), Token::Symbol('='), Token::String(Some(value))]
                            if name == "$out_dir" =>
                        {
                            Some(if value.is_empty() {
                                ".".into()
                            } else {
                                value.clone()
                            })
                        }
                        [Token::Word(target), Token::Symbol('='), Token::Word(source)]
                            if target.starts_with('$')
                                && target != "$out_dir"
                                && source == "$out_dir" =>
                        {
                            // A common rc sets $aux_dir = $out_dir. Reading
                            // the output value does not change that value.
                            output
                        }
                        _ => None,
                    };
                }
                statement.clear();
                mentions_output = false;
                continue;
            }
            ch => Token::Symbol(ch),
        };
        // Only a three-token assignment can qualify. Keep a fourth token to
        // distinguish all longer statements without unbounded allocation.
        if statement.len() < 4 {
            statement.push(token);
        }
        if ch == '}' && depth == 0 {
            if mentions_output {
                output = None;
            }
            statement.clear();
            mentions_output = false;
        }
    }
    if depth != 0 || mentions_output {
        return None;
    }
    output.filter(|value| !value.contains(['\0', '\n', '\r']))
}

/// Return a decoded literal, or None for a Perl string with interpolation.
fn quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    quote: char,
) -> Option<Option<String>> {
    let mut value = String::new();
    let mut literal = quote != '`';
    while let Some(ch) = chars.next() {
        if ch == quote {
            return Some(literal.then_some(value));
        }
        if ch == '\\' {
            let escaped = chars.next()?;
            match escaped {
                '\\' => value.push('\\'),
                ch if ch == quote => value.push(ch),
                '$' | '@' if quote == '"' => value.push(escaped),
                _ if quote == '\'' => {
                    value.push('\\');
                    value.push(escaped);
                }
                _ => literal = false,
            }
        } else {
            if quote == '"' && matches!(ch, '$' | '@') {
                literal = false;
            }
            value.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_literal_assignments_comments_and_last_unconditional_value() {
        for (source, expected) in [
            ("$out_dir = 'build';", "build"),
            ("$out_dir\n = \"输出 build\"; # comment", "输出 build"),
            (
                "# $out_dir = 'wrong';\n$aux_dir = 'aux'; $out_dir = 'build';",
                "build",
            ),
            ("$out_dir = 'first'; $out_dir = 'last';", "last"),
            ("$out_dir = 'build'; $aux_dir = $out_dir;", "build"),
            ("$out_dir = '';", "."),
            ("$out_dir = 'build#files';", "build#files"),
            (r"$out_dir = 'it\'s\\build';", "it's\\build"),
            (
                r#"$out_dir = "build\$literal\@files";"#,
                "build$literal@files",
            ),
            ("$text = '$out_dir = wrong'; $out_dir = 'build';", "build"),
            (
                "if (1) { $out_dir = 'unknown'; } $out_dir = 'build';",
                "build",
            ),
        ] {
            assert_eq!(
                literal_output_directory(source).as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn does_not_guess_dynamic_conditional_or_quoted_perl_code() {
        for source in [
            "# $out_dir = 'build';",
            "$out_dir = 'build'",
            "$out_dir = 'build'; $out_dir .= '/more';",
            "$out_dir = 'build' if $flag;",
            "$out_dir = 'first'; if ($flag) { $out_dir = 'build'; }",
            "$out_dir = 'first'; sub setup { $out_dir = 'build'; }",
            "$out_dir = 'build' . '/more';",
            "$out_dir = $ENV{'BUILD_DIR'};",
            "$out_dir = 'first'; $out_dir = $ENV{'BUILD_DIR'};",
            "$out_dir = \"$ENV{HOME}/build\";",
            "$out_dir = `echo build`;",
            "print q{$out_dir = 'build';};",
            "print <<'EOF';\n$out_dir = 'build';\nEOF",
            "=pod\n$out_dir = 'build';\n=cut",
            "$out_dir = 'build'; __DATA__\n$out_dir = 'other';",
        ] {
            let expected = source.contains("__DATA__").then_some("build");
            assert_eq!(
                literal_output_directory(source).as_deref(),
                expected,
                "{source}"
            );
        }
    }
}
