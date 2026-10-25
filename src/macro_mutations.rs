//! Possible replacement-text mutations; never infer that a patch succeeds.
use crate::syntax;

pub(crate) const COMMANDS: &[&str] = &[
    "pretocmd",
    "apptocmd",
    "patchcmd",
    "appto",
    "eappto",
    "gappto",
    "xappto",
    "preto",
    "epreto",
    "gpreto",
    "xpreto",
    "csappto",
    "cseappto",
    "csgappto",
    "csxappto",
    "cspreto",
    "csepreto",
    "csgpreto",
    "csxpreto",
    "g@addto@macro",
];

pub(crate) fn parameters(name: &str) -> Option<&'static [(&'static str, bool)]> {
    Some(match name {
        "patchcmd" => &[
            ("prefix", true),
            ("command", false),
            ("search", false),
            ("replacement", false),
            ("success", false),
            ("failure", false),
        ],
        "pretocmd" | "apptocmd" => &[
            ("command", false),
            ("code", false),
            ("success", false),
            ("failure", false),
        ],
        name if COMMANDS.contains(&name) => &[("command", false), ("code", false)],
        _ => return None,
    })
}

pub(crate) fn code(label: &str) -> bool {
    matches!(label, "code" | "replacement" | "success" | "failure")
}

pub(crate) fn target(source: &str, mut at: usize, name: &str, work: &mut usize) -> Option<String> {
    at = syntax::skip_space(source, at);
    if name == "patchcmd" && source.as_bytes().get(at) == Some(&b'[') {
        let end = syntax::optional_note_end(source, at)?;
        *work = work.saturating_add((end - at) * 2);
        if *work > 32 * 1024 * 1024 {
            return None;
        }
        let prefix = &source[at + 1..end - 1];
        let mut cursor = syntax::skip_space(prefix, 0);
        while cursor < prefix.len() {
            let token = syntax::control_word(prefix, cursor)?;
            if !matches!(
                &prefix[token.clone()],
                "\\long" | "\\protected" | "\\global" | "\\outer"
            ) {
                return None;
            }
            cursor = syntax::skip_space(prefix, token.end);
        }
        at = syntax::skip_space(source, end);
    }
    if name.starts_with("cs") {
        let (argument, _) = syntax::group(source, at, b'{', b'}')?;
        *work = work.saturating_add(argument.outer_span.len());
        return syntax::is_static_key(&argument.value).then_some(argument.value);
    }
    let (bounded, at, end) = if source.as_bytes().get(at) == Some(&b'{') {
        let (argument, _) = syntax::group(source, at, b'{', b'}')?;
        *work = work.saturating_add(argument.outer_span.len());
        (
            &source[..argument.span.end],
            syntax::skip_space(source, argument.span.start),
            Some(argument.span.end),
        )
    } else {
        (source, at, None)
    };
    let word = syntax::control_word(bounded, at)?;
    if end.is_some_and(|end| syntax::skip_space(bounded, word.end) != end)
        || source[word.clone()].starts_with("\\csname")
    {
        return None;
    }
    Some(source[word.start + 1..word.end].into())
}
