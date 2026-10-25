//! Literal keyword lists follow TeX tokenization and native CSV item spaces.
//!
//! Keep trailing spaces: native control-sequence names and keyword membership
//! distinguish `topic` from `topic `. No expansion or nested groups are proved.
//! Unicode letters, numbers and basic combining accents keep their exact
//! spelling. In particular, a source NFD name need not match Biber's NFC name.
pub(crate) fn list(source: &str) -> Option<Vec<String>> {
    if source.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut tokens = String::with_capacity(source.len());
    // 0: beginning of physical line, 1: after a character, 2: skipping blanks.
    // The argument's opening brace was already read, hence initial state 1.
    let mut state = 1u8;
    for physical in source.split_inclusive('\n') {
        let newline = physical.ends_with('\n');
        let line = if newline {
            physical[..physical.len() - 1].trim_end_matches([' ', '\t', '\r'])
        } else {
            // This substring ends before the closing brace, not at a physical
            // end of line: its spaces must not be trimmed.
            physical
        };
        let (line, commented) = line
            .split_once('%')
            .map_or((line, false), |(line, _)| (line, true));
        for ch in line.chars() {
            match ch {
                ' ' | '\t' => {
                    if state == 1 {
                        tokens.push(' ');
                        state = 2;
                    }
                }
                ch if ch.is_ascii_graphic() && !"\\{}#~^".contains(ch)
                    || !ch.is_ascii()
                        && (ch.is_alphanumeric() || ('\u{300}'..='\u{36f}').contains(&ch)) =>
                {
                    tokens.push(ch);
                    state = 1;
                }
                _ => return None,
            }
        }
        if commented {
            state = 0;
        } else if newline {
            match state {
                0 => return None, // A blank line introduces an unproved \par.
                1 => tokens.push(' '),
                _ => {}
            }
            state = 0;
        }
    }
    let mut names = Vec::new();
    for name in tokens
        .split(',')
        .map(|name| name.trim_start_matches(' '))
        .filter(|name| !name.is_empty())
    {
        if names.len() >= 100_000 {
            return None;
        }
        names.push(name.to_string());
    }
    Some(names)
}
