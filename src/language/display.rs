//! Human-readable source paths, separate from protocol URI identities.
use lsp_types::Url;

pub(crate) fn path(uri: &Url) -> String {
    uri.to_file_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| uri.path().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_paths_decode_unicode_and_spaces_once_without_changing_the_uri() {
        let original = "/tmp/中文目录/with space/字面%20/文献😀.bib";
        let uri = Url::from_file_path(original).unwrap();
        let encoded = uri.as_str().to_owned();
        assert_eq!(path(&uri), original);
        assert_eq!(uri.as_str(), encoded);
        assert!(encoded.contains("%E4%B8%AD"));
        assert!(encoded.contains("%2520"));
    }

    #[test]
    fn non_file_documents_keep_their_existing_path_display() {
        let uri = Url::parse("untitled:chapter.tex").unwrap();
        assert_eq!(path(&uri), uri.path());
    }
}
