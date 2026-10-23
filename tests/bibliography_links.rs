use latex_lsp::{
    navigation,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::{DocumentLink, Position, Url};

fn links(source: &str) -> (Document, Vec<DocumentLink>) {
    let uri = Url::parse("file:///bibliography-links/refs.bib").unwrap();
    let doc = Document::new(&uri, source.into(), Some(1));
    let mut workspace = Workspace::default();
    workspace.documents.insert(uri.clone(), doc.clone());
    (doc, navigation::document_links(&workspace, &uri))
}

fn values(doc: &Document, links: &[DocumentLink]) -> Vec<(String, String)> {
    links
        .iter()
        .map(|link| {
            (
                doc.text.source[doc.text.offset(link.range.start).unwrap()
                    ..doc.text.offset(link.range.end).unwrap()]
                    .into(),
                link.target.as_ref().unwrap().as_str().into(),
            )
        })
        .collect()
}

#[test]
fn standalone_bibliographies_link_braced_and_quoted_fields_with_exact_utf16_ranges() {
    let source = concat!(
        "@book(中文😀,\r\n",
        "  URL = {\r\n https://example.org/中😀?q=one%20two&x=1#part \r\n },\r\n",
        "  DOI = \"  DOI: 10.1234.56/abc  \",\r\n",
        "  title = {A title}, author = {A. Writer}, year = 2026\r\n",
        ")\r\n",
    );
    let (doc, links) = links(source);
    assert_eq!(
        values(&doc, &links),
        [
            (
                "https://example.org/中😀?q=one%20two&x=1#part".into(),
                "https://example.org/%E4%B8%AD%F0%9F%98%80?q=one%20two&x=1#part".into()
            ),
            (
                "DOI: 10.1234.56/abc".into(),
                "https://doi.org/10.1234.56/abc".into()
            ),
        ]
    );
    assert_eq!(links[0].range.start, Position::new(2, 1));
    assert_eq!(links[0].tooltip.as_deref(), Some("Open bibliography URL"));
    assert_eq!(links[1].tooltip.as_deref(), Some("Open DOI"));
    assert!(doc.syntax.citations[0].detail.contains("Title: A title"));
    assert!(doc.syntax.citations[0].detail.contains("Year: 2026"));
    let (doc, links) = self::links("@book{😀, url={https://example.org/x}}");
    assert_eq!(links[0].range.start.character, 15);
    assert_eq!(values(&doc, &links)[0].0, "https://example.org/x");
}

#[test]
fn bare_dois_encode_reserved_characters_and_unicode_as_identifier_data() {
    let (doc, links) = links(
        r#"@article{one, doi={10.1234/a%2F?x#frag/α(β)}}
@article{two, doi="doi:10.5678/a\_b\#c\%d\&e\$f"}"#,
    );
    assert_eq!(
        values(&doc, &links),
        [
            (
                "10.1234/a%2F?x#frag/α(β)".into(),
                "https://doi.org/10.1234/a%252F%3Fx%23frag/%CE%B1%28%CE%B2%29".into()
            ),
            (
                r"doi:10.5678/a\_b\#c\%d\&e\$f".into(),
                "https://doi.org/10.5678/a_b%23c%25d%26e%24f".into()
            ),
        ]
    );
    for link in links {
        let target = link.target.unwrap();
        assert_eq!(target.query(), None);
        assert_eq!(target.fragment(), None);
    }
}

#[test]
fn resolver_urls_preserve_their_uri_encoding_query_and_fragment() {
    let (doc, links) = links(
        r#"@book{one,
doi={https://DOI.ORG/10.1234/a%23b?download=1#page=2},
url={https://example.org/a\_b?q=1\&x=2\#part}}
@book{two, DOI="http://dx.doi.org/10.5678/AbC"}"#,
    );
    let values = values(&doc, &links);
    assert_eq!(
        values[0].1,
        "https://doi.org/10.1234/a%23b?download=1#page=2"
    );
    assert_eq!(values[1].1, "https://example.org/a_b?q=1&x=2#part");
    assert_eq!(values[2].1, "http://dx.doi.org/10.5678/AbC");
}

#[test]
fn doi_dot_segments_remain_identifier_data_instead_of_being_normalized_as_url_paths() {
    for (doi, target) in [
        ("10.1234/a/../b", "https://doi.org/10.1234%2Fa%2F..%2Fb"),
        ("10.1234/.", "https://doi.org/10.1234%2F."),
        ("10.1234/..", "https://doi.org/10.1234%2F.."),
        ("10.1234/a/./b", "https://doi.org/10.1234%2Fa%2F.%2Fb"),
    ] {
        let (_, result) = links(&format!("@book{{key,doi={{{doi}}}}}"));
        assert_eq!(result[0].target.as_ref().unwrap().as_str(), target);
        let (_, result) = links(&format!("@book{{key,doi={{{target}}}}}"));
        assert_eq!(result[0].target.as_ref().unwrap().as_str(), target);
    }
    assert_eq!(
        links("@book{key,doi={https://doi.org/10%2E1234%2Fencoded}}")
            .1
            .len(),
        1
    );
    assert!(links("@book{key,doi={https://doi.org/10.1234/a%20b}}")
        .1
        .is_empty());
    assert!(links("@book{key,doi={https://doi.org/10.1234/%ff}}")
        .1
        .is_empty());
}

#[test]
fn ambiguous_duplicate_fields_are_suppressed_per_entry_and_per_field() {
    let (doc, links) = links(
        r#"@book{one,
url={https://example.org/first}, URL={https://example.org/second},
doi={10.1234/good}, doi=unknown, DOI={10.1234/third}}
@book{two, url=unknown, url={https://example.org/hidden}, doi={10.1234/visible}}
@book{three, url={https://example.org/visible}, doi={bad}, doi={10.1234/hidden}}
@book{four, url={https://example.org/one}, doi={10.1234/one}}"#,
    );
    let targets: Vec<_> = values(&doc, &links)
        .into_iter()
        .map(|(_, target)| target)
        .collect();
    assert_eq!(
        targets,
        [
            "https://doi.org/10.1234/visible",
            "https://example.org/visible",
            "https://example.org/one",
            "https://doi.org/10.1234/one"
        ]
    );
}

#[test]
fn macros_concatenations_comments_titles_and_special_entries_do_not_create_links() {
    let source = r#"% @book{commented, url={https://example.org/commented}}
@comment{url={https://example.org/comment}, doi={10.1234/comment}}
@string{url="https://example.org/string"}
@preamble{"doi={10.1234/preamble}"}
@book{one, title={url={https://example.org/title}},
url={https://example.org/prefix} # "suffix", doi="10.1234/" # code}
@book{two, url=baseurl, doi=identifier}
@book{three, url={\url{https://example.org/wrapped}}, doi={\identifier}}
@book{four, url={https://example.org/{nested}}, doi={10.1234/{nested}}}
@book{five, url={https://example.org/with space}, doi={10.1234/with space}}
@book{six, url={javascript:alert(1)}, doi={https://example.org/10.1234/foreign}}
@book{seven, url={https://example.org/\unknown}, doi={10.1234/\unknown}}"#;
    assert!(links(source).1.is_empty());
}

#[test]
fn incomplete_values_are_not_links_but_complete_fields_in_unfinished_entries_are() {
    for source in [
        "@book{one, url={https://example.org/unfinished",
        "@book{one, url=\"https://example.org/unfinished",
        "@book{one, doi={10.1234/unfinished",
        "@book{one, doi=\"10.1234/unfinished",
    ] {
        assert!(links(source).1.is_empty(), "{source}");
    }
    for source in [
        "@book{one, url={https://example.org/complete}",
        "@book{one, url=\"https://example.org/complete\"",
    ] {
        assert_eq!(links(source).1.len(), 1, "{source}");
    }
}

#[test]
fn only_structurally_valid_dois_and_supported_absolute_urls_are_links() {
    for doi in [
        "",
        "10.1234/",
        "10./suffix",
        "11.1234/suffix",
        "10.12x4/suffix",
        "10.1234..5/suffix",
        "10.1234",
        "https://doi.org/wrong",
        "https://user@doi.org/10.1234/suffix",
        "https://doi.org:8443/10.1234/suffix",
    ] {
        assert!(
            links(&format!("@book{{key,doi={{{doi}}}}}")).1.is_empty(),
            "{doi}"
        );
    }
    for url in [
        "",
        "relative/path",
        "//example.org/path",
        "javascript:alert(1)",
        "data:text/plain,hello",
        "command:run",
        "https://exa mple.org",
    ] {
        assert!(
            links(&format!("@book{{key,url={{{url}}}}}")).1.is_empty(),
            "{url}"
        );
    }
    for url in [
        "https://example.org/",
        "http://example.org/",
        "mailto:author@example.org",
        "ftp://example.org/paper.pdf",
        "file:///tmp/paper.pdf",
    ] {
        assert_eq!(
            links(&format!("@book{{key,url={{{url}}}}}")).1.len(),
            1,
            "{url}"
        );
    }
}

#[test]
fn oversized_link_values_are_ignored_without_losing_the_next_field() {
    let source = format!(
        "@book{{key,url={{https://example.org/{}}},doi={{10.1234/good}}}}",
        "x".repeat(17_000)
    );
    let (doc, links) = links(&source);
    assert_eq!(
        values(&doc, &links),
        [("10.1234/good".into(), "https://doi.org/10.1234/good".into())]
    );
}

#[test]
fn shared_bibliographies_produce_the_same_links_without_selecting_a_project_root() {
    let directory = tempfile::tempdir().unwrap();
    let first = Url::from_file_path(directory.path().join("first.tex")).unwrap();
    let second = Url::from_file_path(directory.path().join("second.tex")).unwrap();
    let bib = Url::from_file_path(directory.path().join("refs.bib")).unwrap();
    let source = "@book{key,url={https://example.org/paper},doi={10.1234/paper}}";
    std::fs::write(bib.to_file_path().unwrap(), source).unwrap();
    for uri in [&first, &second] {
        std::fs::write(
            uri.to_file_path().unwrap(),
            r"\documentclass{article}\bibliography{refs}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let result = navigation::document_links(&workspace, &bib);
    assert_eq!(result.len(), 2);
    assert_eq!(result, links(source).1);
}
