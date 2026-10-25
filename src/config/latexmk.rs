//! Read literal project build settings without executing a Perl rc file.
use std::{fs::File, io::Read, path::Path};

use super::Engine;

const MAX_RC_SIZE: u64 = 64 * 1024;

pub(super) struct Settings {
    pub output_directory: Result<Option<String>, String>,
    pub engine: Result<Option<Engine>, String>,
    pub auxiliary_directory: Result<Option<String>, String>,
    pub final_output_directory: Result<Option<String>, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_directory: Ok(None),
            engine: Ok(None),
            auxiliary_directory: Ok(None),
            final_output_directory: Ok(None),
        }
    }
}

pub(super) fn read(path: &Path) -> Settings {
    crate::diagnostics::diagnostic_inputs::observe(path);
    let read = || -> Result<String, String> {
        let file = File::open(path).map_err(|error| error.to_string())?;
        if !file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("not a regular file".into());
        }
        let mut source = String::new();
        file.take(MAX_RC_SIZE + 1)
            .read_to_string(&mut source)
            .map_err(|error| error.to_string())?;
        if source.len() as u64 > MAX_RC_SIZE {
            return Err("configuration exceeds 64 KiB".into());
        }
        Ok(source)
    };
    match read() {
        Ok(source) => Settings {
            output_directory: output_setting(&source),
            engine: engine_setting(&source),
            auxiliary_directory: directory_setting(&source, "$aux_dir"),
            final_output_directory: directory_setting(&source, "$out2_dir"),
        },
        Err(error) => Settings {
            output_directory: Err(error.clone()),
            engine: Err(error.clone()),
            auxiliary_directory: Err(error.clone()),
            final_output_directory: Err(error),
        },
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    String(Option<String>),
    Symbol(char),
}

#[cfg(test)]
fn literal_output_directory(source: &str) -> Option<String> {
    output_setting(source).ok().flatten()
}

#[cfg(test)]
fn literal_engine(source: &str) -> Option<Engine> {
    engine_setting(source).ok().flatten()
}

fn output_setting(source: &str) -> Result<Option<String>, String> {
    directory_setting(source, "$out_dir")
}

fn directory_setting(source: &str, setting: &str) -> Result<Option<String>, String> {
    literal_setting(source, setting)?
        .map(|value| {
            if value.contains(['\0', '\n', '\r']) {
                return Err("invalid directory characters".into());
            }
            Ok(if value.is_empty() && setting == "$out_dir" {
                ".".into()
            } else {
                value
            })
        })
        .transpose()
}

fn engine_setting(source: &str) -> Result<Option<Engine>, String> {
    literal_setting(source, "$pdf_mode")?
        .map(|value| match value.as_str() {
            "1" => Ok(Engine::PdfLatex),
            "4" => Ok(Engine::LuaLatex),
            "5" => Ok(Engine::XeLatex),
            _ => Err(format!("unsupported $pdf_mode {value:?}")),
        })
        .transpose()
}

fn literal_setting(source: &str, setting: &str) -> Result<Option<String>, String> {
    let unknown =
        || format!("cannot statically determine {setting}; dynamic or unsupported Perl syntax");
    let mut chars = source.chars().peekable();
    let mut statement = Vec::new();
    let mut mentions_output = false;
    let mut depth = 0usize;
    let mut output = Ok(None);
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
            '\'' | '"' | '`' => Token::String(quoted(&mut chars, ch).ok_or_else(unknown)?),
            // Perl's quote-like operators, regexes, heredocs and POD can
            // contain apparent assignments. Do not guess their contents.
            '/' | '\\' => return Err(unknown()),
            '<' if chars.peek() == Some(&'<') => return Err(unknown()),
            '=' if chars.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) => return Err(unknown()),
            ch if ch.is_ascii_alphanumeric() || matches!(ch, '$' | '_') => {
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
                    "q" | "qq"
                        | "qw"
                        | "qx"
                        | "qr"
                        | "m"
                        | "s"
                        | "tr"
                        | "y"
                        | "do"
                        | "require"
                        | "eval"
                        | "process_rc_file"
                        | "$auto_rc_use"
                ) {
                    return Err(unknown());
                }
                // Calls may mutate settings through a function defined in an
                // earlier rc layer, even when this statement never names them.
                if !word.starts_with('$')
                    && !matches!(
                        word.as_str(),
                        "if" | "unless" | "for" | "foreach" | "while" | "sub"
                    )
                {
                    let mut following = chars.clone();
                    while following.peek().is_some_and(|ch| ch.is_whitespace()) {
                        following.next();
                    }
                    if following.peek() == Some(&'(') {
                        return Err(unknown());
                    }
                }
                if word == setting {
                    mentions_output = true;
                    if depth != 0 {
                        // Conditional assignments and subroutines are not
                        // unconditional project settings.
                        output = Err(unknown());
                    }
                }
                Token::Word(word)
            }
            '{' | '(' | '[' => {
                depth += 1;
                Token::Symbol(ch)
            }
            '}' | ')' | ']' => {
                depth = depth.checked_sub(1).ok_or_else(unknown)?;
                Token::Symbol(ch)
            }
            ';' if depth == 0 => {
                if matches!(statement.first(), Some(Token::Word(word)) if !word.starts_with('$')) {
                    return Err(unknown());
                }
                if mentions_output {
                    output = match statement.as_slice() {
                        [Token::Word(name), Token::Symbol('='), Token::String(Some(value))]
                            if name == setting =>
                        {
                            Ok(Some(value.clone()))
                        }
                        [Token::Word(name), Token::Symbol('='), Token::Word(value)]
                            if name == setting
                                && setting == "$pdf_mode"
                                && value.chars().all(|ch| ch.is_ascii_digit()) =>
                        {
                            Ok(Some(value.clone()))
                        }
                        [Token::Word(name), Token::Symbol('='), Token::Word(reference)]
                            if name == setting
                                && matches!(setting, "$aux_dir" | "$out2_dir")
                                && reference == "$out_dir" =>
                        {
                            output_setting(source)
                        }
                        [Token::Word(target), Token::Symbol('='), Token::Word(source)]
                            if target.starts_with('$')
                                && target != setting
                                && source == setting =>
                        {
                            // A common rc sets $aux_dir = $out_dir. Reading
                            // the output value does not change that value.
                            output
                        }
                        _ => Err(unknown()),
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
                output = Err(unknown());
            }
            statement.clear();
            mentions_output = false;
        }
    }
    if depth != 0 || mentions_output {
        return Err(unknown());
    }
    output
}

/// Return a decoded literal, or None for a Perl string with interpolation.
pub(super) fn quoted(
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
    fn reads_pdf_modes_without_guessing_dynamic_or_conditional_assignments() {
        for (source, expected) in [
            ("$pdf_mode = 1;", Some(Engine::PdfLatex)),
            ("$pdf_mode = 4;", Some(Engine::LuaLatex)),
            ("$pdf_mode = 5;", Some(Engine::XeLatex)),
            ("$pdf_mode = '5';", Some(Engine::XeLatex)),
            ("$pdf_mode = 1; $pdf_mode = 5;", Some(Engine::XeLatex)),
            ("$pdf_mode = 5; $other = $pdf_mode;", Some(Engine::XeLatex)),
            (
                "$ENV{'TEXINPUTS'} = '.:./styles//:' . $ENV{'TEXINPUTS'};\n\
                 # $pdf_mode = 1;\n$pdf_mode = 5;\n\
                 $postscript_mode = $dvi_mode = 0;\n$out_dir = \"build\";\n\
                 $xdvipdfmx = 'xdvipdfmx -E -o %D %O %S';\n\
                 @default_files = ('main.tex');",
                Some(Engine::XeLatex),
            ),
            ("# $pdf_mode = 5;", None),
            ("$text = '$pdf_mode = 5;';", None),
            ("$pdf_mode = 5", None),
            ("$pdf_mode = 0;", None),
            ("$pdf_mode = 2;", None),
            ("$pdf_mode = 3;", None),
            ("$pdf_mode = 50;", None),
            ("$pdf_mode = 1 + 4;", None),
            ("$pdf_mode = $ENV{'MODE'};", None),
            ("$pdf_mode = 5 if $flag;", None),
            ("$pdf_mode = 5; if ($flag) { $pdf_mode = 1; }", None),
            ("$pdf_mode = 5; $pdf_mode += 1;", None),
            ("$pdf_mode = 5; $pdf_mode = unknown();", None),
            ("$pdf_mode = 5; configure_engine();", None),
            ("$pdf_mode = 5; configure_engine 'project';", None),
        ] {
            assert_eq!(literal_engine(source), expected, "{source}");
        }
    }

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
