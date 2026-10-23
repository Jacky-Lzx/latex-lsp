//! Native IDS conflicts distinguish the accepted owner from the skipped alias.
use super::{biber_keys::key_name, Context, Parsed};
use crate::syntax;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tower_lsp::lsp_types::*;

const MAX_RELATED: usize = 16;

pub(crate) struct Record<'a> {
    alias: &'a str,
    kind: Kind<'a>,
}

#[derive(Clone, Copy)]
enum Kind<'a> {
    Duplicate(&'a str),
    RealKey,
    SelfAlias,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let (alias, kind) = if let Some(body) = message.strip_prefix("Entry alias '") {
        if let Some(alias) = body.strip_suffix("' is also a real entry key, skipping ...") {
            (alias, Kind::RealKey)
        } else {
            let (alias, owner) = body
                .strip_suffix("', skipping ...")?
                .rsplit_once("' already has an alias '")?;
            if !syntax::is_static_citation_key(owner) {
                return None;
            }
            (alias, Kind::Duplicate(owner))
        }
    } else if let Some(body) = message.strip_prefix("Citekey alias '") {
        (
            body.strip_suffix("' is also a real entry key, skipping ...")?,
            Kind::RealKey,
        )
    } else {
        (
            message
                .strip_prefix("BAD RECURSION! Entry alias '")?
                .strip_suffix("' is identical to the entry key, skipping ...")?,
            Kind::SelfAlias,
        )
    };
    syntax::is_static_citation_key(alias).then_some(Record { alias, kind })
}

struct Alias {
    owner: Arc<str>,
    range: Range,
}

#[derive(Default)]
struct Candidates {
    count: usize,
    aliases: Vec<Alias>,
}

pub(super) struct Locations {
    aliases: BTreeMap<String, Candidates>,
}

impl Locations {
    fn new(parsed: &Parsed) -> Self {
        let source = &parsed.text.source;
        let mut aliases = BTreeMap::<String, Candidates>::new();
        let mut declarations = parsed.syntax.bibliography_aliases.iter().peekable();
        let mut byte = 0;
        let mut position = Position::new(0, 0);
        let mut at = |target| {
            for character in source[byte..target].chars() {
                if character == '\n' {
                    position.line += 1;
                    position.character = 0;
                } else {
                    position.character += character.len_utf16() as u32;
                }
            }
            byte = target;
            position
        };
        for entry in &parsed.syntax.bibliography_completion.types {
            let (Some(body), Some(key)) = (&entry.entry_span, &entry.key) else {
                continue;
            };
            let owner: Arc<str> = key_name(&source[key.clone()]).into();
            while declarations
                .peek()
                .is_some_and(|alias| alias.span.start < body.end)
            {
                let alias = declarations.next().unwrap();
                let candidates = aliases.entry(key_name(&alias.name)).or_default();
                candidates.count += 1;
                if candidates.aliases.len() < MAX_RELATED {
                    candidates.aliases.push(Alias {
                        owner: owner.clone(),
                        range: Range::new(at(alias.span.start), at(alias.span.end)),
                    });
                }
            }
        }
        Self { aliases }
    }
}

impl Parsed {
    fn biber_alias_locations(&self) -> &Locations {
        self.biber_alias_locations
            .get_or_init(|| Locations::new(self))
    }
}

impl Context {
    pub(crate) fn biber_alias_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"alias":warning.alias,"unresolvedSource":true});
        data["reason"] = match warning.kind {
            Kind::Duplicate(owner) => {
                data["owner"] = owner.into();
                "duplicate-alias"
            }
            Kind::RealKey => "entry-key-collision",
            Kind::SelfAlias => "self-alias",
        }
        .into();
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                "conflicting-bibliography-alias".into(),
            )),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        // There is no native filename or skipped owner. Verify every prior
        // input before selecting a declaration; never infer warning order.
        let inputs = self.biber_sources.inputs().unwrap_or_default().to_vec();
        let documents = inputs
            .iter()
            .map(|path| self.document(path).map(|parsed| (path, parsed)))
            .collect::<Option<Vec<_>>>();
        if let Some(documents) = documents.filter(|documents| !documents.is_empty()) {
            let alias_name = key_name(warning.alias);
            let mut aliases = Vec::new();
            let mut count = 0;
            for (path, parsed) in &documents {
                if let Some(candidates) = parsed.biber_alias_locations().aliases.get(&alias_name) {
                    count += candidates.count;
                    for alias in candidates.aliases.iter().take(MAX_RELATED - aliases.len()) {
                        aliases.push((*path, alias));
                    }
                }
            }
            let keys = |key: &str| {
                documents
                    .iter()
                    .flat_map(|(path, parsed)| {
                        parsed.biber_locations().key(key).iter().map(move |range| {
                            Location::new(Url::from_file_path(path).unwrap(), *range)
                        })
                    })
                    .take(MAX_RELATED)
                    .collect::<Vec<_>>()
            };
            let owner = match warning.kind {
                Kind::Duplicate(owner) => key_name(owner),
                Kind::RealKey | Kind::SelfAlias => alias_name.clone(),
            };
            let primary_keys = keys(&owner);
            let selected = if primary_keys.len() == 1 && count <= MAX_RELATED {
                let candidates: Vec<_> = match warning.kind {
                    Kind::Duplicate(_) => {
                        let accepted = aliases
                            .iter()
                            .filter(|(_, alias)| alias.owner.as_ref() == owner)
                            .count();
                        aliases
                            .iter()
                            .filter(|(_, alias)| accepted == 1 && alias.owner.as_ref() != owner)
                            .collect()
                    }
                    Kind::RealKey => aliases
                        .iter()
                        .filter(|(_, alias)| alias.owner.as_ref() != owner)
                        .collect(),
                    Kind::SelfAlias => aliases
                        .iter()
                        .filter(|(_, alias)| alias.owner.as_ref() == owner)
                        .collect(),
                };
                match candidates.as_slice() {
                    [(path, alias)] if keys(&alias.owner).len() == 1 => {
                        Some(Location::new(Url::from_file_path(path).ok()?, alias.range))
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(selected) = &selected {
                uri = selected.uri.clone();
                diagnostic.range = selected.range;
                data.as_object_mut()?.remove("unresolvedSource");
            } else {
                data["unresolvedAlias"] = true.into();
            }
            let mut related = Vec::new();
            for location in primary_keys
                .into_iter()
                .chain(aliases.iter().map(|(path, alias)| {
                    Location::new(Url::from_file_path(path).unwrap(), alias.range)
                }))
            {
                if selected.as_ref() == Some(&location)
                    || related
                        .iter()
                        .any(|item: &DiagnosticRelatedInformation| item.location == location)
                {
                    continue;
                }
                related.push(DiagnosticRelatedInformation {
                    location,
                    message: "Bibliography declaration matching Biber's alias conflict".into(),
                });
                if related.len() == MAX_RELATED {
                    break;
                }
            }
            if !related.is_empty() {
                diagnostic.related_information = Some(related);
            }
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
