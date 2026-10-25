//! Bounded, connection-local semantic token snapshots for LSP delta requests.
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

// A token has five u32 fields. Bound both token storage and per-file overhead.
const MAX_TOKENS: usize = 65_536;
const MAX_DOCUMENTS: usize = 16;

struct Snapshot {
    sequence: u64,
    tokens: SemanticTokens,
}

#[derive(Default)]
pub(crate) struct Cache {
    snapshots: BTreeMap<Url, Snapshot>,
    sequence: u64,
    token_count: usize,
}

impl Cache {
    pub(crate) fn remove(&mut self, uri: &Url) {
        if let Some(snapshot) = self.snapshots.remove(uri) {
            self.token_count -= snapshot.tokens.data.len();
        }
    }

    pub(crate) fn full(&mut self, uri: Url, mut tokens: SemanticTokens) -> SemanticTokens {
        self.remove(&uri);
        tokens.result_id = None;
        // Large documents still receive a full response; do not retain them.
        if tokens.data.len() > MAX_TOKENS {
            return tokens;
        }
        while self.snapshots.len() >= MAX_DOCUMENTS
            || self.token_count + tokens.data.len() > MAX_TOKENS
        {
            let Some(oldest) = self
                .snapshots
                .iter()
                .min_by_key(|(_, snapshot)| snapshot.sequence)
                .map(|(uri, _)| uri.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        self.sequence += 1;
        tokens.result_id = Some(format!("semantic-{}", self.sequence));
        self.token_count += tokens.data.len();
        self.snapshots.insert(
            uri,
            Snapshot {
                sequence: self.sequence,
                tokens: tokens.clone(),
            },
        );
        tokens
    }

    pub(crate) fn delta(
        &mut self,
        uri: Url,
        previous: &str,
        tokens: SemanticTokens,
    ) -> SemanticTokensFullDeltaResult {
        let edits = (tokens.data.len() <= MAX_TOKENS)
            .then(|| self.snapshots.get(&uri))
            .flatten()
            .filter(|snapshot| snapshot.tokens.result_id.as_deref() == Some(previous))
            .map(|snapshot| difference(&snapshot.tokens.data, &tokens.data));
        let full = self.full(uri, tokens);
        match edits {
            Some(edits) => SemanticTokensFullDeltaResult::TokensDelta(SemanticTokensDelta {
                result_id: full.result_id,
                edits,
            }),
            None => SemanticTokensFullDeltaResult::Tokens(full),
        }
    }
}

fn difference(previous: &[SemanticToken], current: &[SemanticToken]) -> Vec<SemanticTokensEdit> {
    let prefix = previous
        .iter()
        .zip(current)
        .take_while(|(old, new)| old == new)
        .count();
    if prefix == previous.len() && prefix == current.len() {
        return vec![];
    }
    let suffix = previous[prefix..]
        .iter()
        .rev()
        .zip(current[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let inserted = &current[prefix..current.len() - suffix];
    vec![SemanticTokensEdit {
        // LSP edit offsets count integers in the flattened wire array.
        start: (prefix * 5) as u32,
        delete_count: ((previous.len() - prefix - suffix) * 5) as u32,
        data: (!inserted.is_empty()).then(|| inserted.to_vec()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(id: usize) -> Url {
        Url::parse(&format!("file:///semantic-cache/{id}.tex")).unwrap()
    }

    fn tokens(ids: &[u32]) -> SemanticTokens {
        SemanticTokens {
            result_id: None,
            data: ids
                .iter()
                .map(|&id| SemanticToken {
                    delta_line: 0,
                    delta_start: id,
                    length: 1,
                    token_type: 0,
                    token_modifiers_bitset: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn delta_edits_reconstruct_insertions_deletions_replacements_and_unchanged_tokens() {
        for (previous, current) in [
            (vec![], vec![]),
            (vec![], vec![1, 2]),
            (vec![1, 2], vec![]),
            (vec![1, 2, 3], vec![1, 2, 3]),
            (vec![1, 2, 3], vec![4, 1, 2, 3]),
            (vec![1, 2, 3], vec![1, 4, 2, 3]),
            (vec![1, 2, 3], vec![1, 2, 3, 4]),
            (vec![1, 2, 3], vec![1, 3]),
            (vec![1, 2, 3], vec![1, 4, 3]),
            (vec![1, 1, 1], vec![1]),
            (vec![1], vec![1, 1, 1]),
        ] {
            let old = tokens(&previous);
            let new = tokens(&current);
            let edits = difference(&old.data, &new.data);
            let mut wire = serde_json::to_value(&old).unwrap()["data"]
                .as_array()
                .unwrap()
                .clone();
            for edit in edits.iter().rev() {
                let serialized = serde_json::to_value(edit).unwrap();
                let inserted = serialized["data"].as_array().cloned().unwrap_or_default();
                wire.splice(
                    edit.start as usize..(edit.start + edit.delete_count) as usize,
                    inserted,
                );
            }
            assert_eq!(
                serde_json::Value::Array(wire),
                serde_json::to_value(&new).unwrap()["data"]
            );
            assert_eq!(edits.is_empty(), previous == current);
        }
    }

    #[test]
    fn snapshots_are_isolated_by_uri_and_only_the_latest_result_can_be_used() {
        let mut cache = Cache::default();
        let first = cache.full(uri(1), tokens(&[1, 2]));
        let id = first.result_id.unwrap();
        assert!(matches!(
            cache.delta(uri(2), &id, tokens(&[1, 3])),
            SemanticTokensFullDeltaResult::Tokens(_)
        ));
        let SemanticTokensFullDeltaResult::TokensDelta(delta) =
            cache.delta(uri(1), &id, tokens(&[1, 3]))
        else {
            panic!()
        };
        assert_eq!(delta.edits.len(), 1);
        assert_eq!(delta.edits[0].start, 5);
        assert_eq!(delta.edits[0].delete_count, 5);
        let next = delta.result_id.unwrap();
        assert_ne!(next, id);
        let SemanticTokensFullDeltaResult::TokensDelta(unchanged) =
            cache.delta(uri(1), &next, tokens(&[1, 3]))
        else {
            panic!()
        };
        assert!(unchanged.edits.is_empty());
        assert!(matches!(
            cache.delta(uri(1), &id, tokens(&[1, 4])),
            SemanticTokensFullDeltaResult::Tokens(_)
        ));
        cache.remove(&uri(1));
        assert!(matches!(
            cache.delta(uri(1), &next, tokens(&[1, 4])),
            SemanticTokensFullDeltaResult::Tokens(_)
        ));
    }

    #[test]
    fn cache_bounds_cover_document_count_total_tokens_replacement_and_oversized_responses() {
        let mut cache = Cache::default();
        let oldest = cache.full(uri(0), tokens(&[])).result_id.unwrap();
        for id in 1..=MAX_DOCUMENTS {
            cache.full(uri(id), tokens(&[]));
        }
        assert_eq!(cache.snapshots.len(), MAX_DOCUMENTS);
        assert!(!cache.snapshots.contains_key(&uri(0)));
        assert!(matches!(
            cache.delta(uri(0), &oldest, tokens(&[])),
            SemanticTokensFullDeltaResult::Tokens(_)
        ));
        cache.full(uri(0), tokens(&vec![1; MAX_TOKENS / 2]));
        cache.full(uri(1), tokens(&vec![2; MAX_TOKENS / 2]));
        assert_eq!(cache.token_count, MAX_TOKENS);
        cache.full(uri(1), tokens(&[2]));
        assert_eq!(cache.token_count, MAX_TOKENS / 2 + 1);
        cache.full(uri(2), tokens(&vec![3; MAX_TOKENS / 2]));
        assert!(cache.token_count <= MAX_TOKENS);
        assert!(!cache.snapshots.contains_key(&uri(0)));
        let oversized = cache.full(uri(2), tokens(&vec![4; MAX_TOKENS + 1]));
        assert!(oversized.result_id.is_none());
        assert!(!cache.snapshots.contains_key(&uri(2)));
        assert_eq!(
            cache.token_count,
            cache
                .snapshots
                .values()
                .map(|snapshot| snapshot.tokens.data.len())
                .sum::<usize>()
        );
    }
}
