//! Literal bibliography links share the field scanner used for citation keys.
use crate::{parser::syntax, project::workspace::Document};
use lsp_types::{DocumentLink, Url};
use std::ops::Range;

#[derive(Clone, Debug)]
pub(crate) struct Link {
    pub span: Range<usize>,
    pub target: Url,
    doi: bool,
}

fn literal_value(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        let character = if character == '\\' {
            match characters.next()? {
                escaped @ ('_' | '%' | '#' | '&' | '$') => escaped,
                _ => return None,
            }
        } else {
            character
        };
        if character.is_whitespace()
            || character.is_control()
            || matches!(character, '{' | '}' | '"')
        {
            return None;
        }
        result.push(character);
    }
    (!result.is_empty()).then_some(result)
}

fn is_doi(value: &str) -> bool {
    let Some((prefix, suffix)) = value.split_once('/') else {
        return false;
    };
    let Some(registrant) = prefix.strip_prefix("10.") else {
        return false;
    };
    // DOI registrant codes can contain numeric subdivisions. Do not impose a
    // publisher-specific digit count or try to establish registry existence.
    !suffix.is_empty()
        && !suffix
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && registrant
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn decoded_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let high = char::from(*bytes.get(at + 1)?).to_digit(16)?;
            let low = char::from(*bytes.get(at + 2)?).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn doi_target(value: &str) -> Option<Url> {
    if let Ok(url) = Url::parse(value) {
        if matches!(url.scheme(), "http" | "https")
            && matches!(url.host_str(), Some("doi.org" | "dx.doi.org"))
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && is_doi(&decoded_path(url.path().strip_prefix('/')?)?)
        {
            return Some(url);
        }
        return None;
    }
    if !is_doi(value) {
        return None;
    }
    // In a bare DOI, '#' and '?' belong to the identifier. Percent signs are
    // literal too; preserve their identity rather than interpreting URI escapes.
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    // A DOI suffix can contain slash-separated '.' or '..' data. Browsers
    // normalize even percent-escaped dot segments. Encode the separators in
    // that case so the resolver receives the original DOI after URI decoding.
    let encode_slashes = value.split('/').any(|part| matches!(part, "." | ".."));
    let mut target = String::from("https://doi.org/");
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || byte == b'/' && !encode_slashes
        {
            target.push(char::from(byte));
        } else {
            target.push('%');
            target.push(char::from(HEX[usize::from(byte >> 4)]));
            target.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    Url::parse(&target).ok()
}

pub(crate) fn field(source: &str, name: &str, start: usize, end: usize) -> Option<Link> {
    if end.saturating_sub(start) > 16_384 {
        return None;
    }
    let argument = syntax::bib_literal_mode(source, start, end, false)?;
    let span = syntax::trimmed_span(source, argument.span);
    let raw = &source[span.clone()];
    let doi = name == "doi";
    let value = if doi
        && raw
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("doi:"))
    {
        raw[4..].trim_start()
    } else {
        raw
    };
    let value = literal_value(value)?;
    let target = if doi {
        doi_target(&value)?
    } else {
        let target = Url::parse(&value).ok()?;
        matches!(
            target.scheme(),
            "http" | "https" | "mailto" | "file" | "ftp"
        )
        .then_some(target)?
    };
    Some(Link { span, target, doi })
}

pub(crate) fn document_links(doc: &Document) -> Vec<DocumentLink> {
    let mut links: Vec<_> = doc
        .syntax
        .bibliography_links
        .iter()
        .map(|link| DocumentLink {
            range: doc.text.range(link.span.start, link.span.end),
            target: Some(link.target.clone()),
            tooltip: Some(
                if link.doi {
                    "Open DOI"
                } else {
                    "Open bibliography URL"
                }
                .into(),
            ),
            data: None,
        })
        .collect();
    links.sort_by_key(|link| (link.range.start, link.range.end));
    links
}
