use super::*;

#[cfg(test)]
mod tex_logs {
    use super::*;

    #[test]
    fn diagnostic_sources_track_closed_nested_and_wrapped_file_entries_not_echoed_messages() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let long = format!("章节-{}.tex", "segment".repeat(20));
        for name in [
            "main.tex",
            "one.tex",
            "two.tex",
            "ignored.tex",
            "package.sty",
            "main.aux",
            long.as_str(),
        ] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let entry = format!("(\"./{long}\")");
        let split = entry
            .char_indices()
            .map(|(at, _)| at)
            .find(|at| *at >= 79)
            .unwrap();
        let log = format!("(./main.tex\n(./one.tex)(./two.tex)\n(./package.sty) (./main.aux)\n{}\n{}\nLaTeX Warning: Do not read (./ignored.tex).\nl.3 (./ignored.tex)\n)\n", &entry[..split], &entry[split..]);
        let (_, sources) = parse_with_sources(&main, &log);
        let expected: BTreeSet<_> = [
            "main.tex",
            "one.tex",
            "two.tex",
            "package.sty",
            long.as_str(),
        ]
        .into_iter()
        .map(|name| Url::from_file_path(root.join(name)).unwrap())
        .collect();
        assert_eq!(sources, expected);
    }

    #[test]
    fn truncated_logs_keep_only_the_root_and_source_entries_actually_observed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        for name in ["main.tex", "observed.tex", "not-seen.tex"] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let log = format!(
            "(./not-se\n{}\n(./observed.tex)\n",
            crate::process::output::TRUNCATED
        );
        let (_, sources) = parse_with_sources(&main, &log);
        assert_eq!(
            sources,
            [
                Url::from_file_path(&main).unwrap(),
                Url::from_file_path(root.join("observed.tex")).unwrap()
            ]
            .into()
        );
    }

    #[test]
    fn duplicate_destination_warnings_select_declarations_and_preserve_shipout_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hypertarget{section.1}{Target}\\label{section.1}\\hyperlink{section.1}{Link}\r\n\\hyperdef[reference-label]{section}% keep\r\n{1}{Target}\r\n";
        std::fs::write(&main, source).unwrap();
        let doc = crate::project::workspace::Document::new(
            &Url::from_file_path(&main).unwrap(),
            source.into(),
            None,
        );
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{section.1}) has been already used, duplicate ignored",
            "warning  (pdf backend): ignoring duplicate destination with the name 'section.1'",
        ] {
            let log = format!("{message}\n<argument> ...shipout:D \\box_use:N \\l_shipout_box\nl.4 \\end{{document}}\nLaTeX Warning: Keep.\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let duplicates: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("duplicate-pdf-destination".into()))).collect();
            assert_eq!(duplicates.len(), 2, "{items:?}");
            assert_eq!(items.len(), 3, "{items:?}");
            for (item, entry) in duplicates.iter().zip(&doc.syntax.anchors) {
                assert_eq!(item.range, doc.text.range(entry.span.start, entry.span.end));
                assert_eq!(item.message, message);
                assert_eq!(item.data, Some(serde_json::json!({"destination":"section.1"})));
            }
        }
    }

    #[test]
    fn duplicate_destination_messages_recover_byte_wraps_without_normalizing_name_spaces() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}", "long".repeat(25));
        std::fs::write(&main, format!("\\hypertarget{{{name}}}{{Target}}\n")).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (ext4): destination with the same identifier (name{",
                "}) has been already used, duplicate ignored",
            ),
            (
                "warning  (pdf backend): ignoring duplicate destination with the name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n<argument> shipout\nl.1 text\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 1, "split {split}: {items:?}");
                assert_eq!(items[0].message, message, "split {split}");
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
            }
            let spaced = format!("{prefix} {name}{suffix}");
            assert!(destination_warning(&spaced).is_none());
        }
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{target}) has been already used",
            "warning  (pdf backend): ignoring duplicate destination with the name '\\dynamic'",
        ] {
            assert!(destination_warning(message).is_none());
        }
    }

    #[test]
    fn destination_warning_byte_wraps_preserve_names_and_stop_at_resource_output() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}.!{}", "long".repeat(25), "tail".repeat(15));
        let source = format!("\\hyperlink{{{name}}}{{Text}}\n");
        std::fs::write(&main, &source).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (dest): name{",
                "} has been referenced but does not exist, replaced by a fixed one",
            ),
            (
                "warning  (pdf backend): unreferenced destination with name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            // Include breaks immediately after punctuation and inside the
            // explanatory words, not just at convenient token boundaries.
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Next warning.\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "split {split}: {items:?}");
                let item = items
                    .iter()
                    .find(|item| item.message != "LaTeX Warning: Next warning.")
                    .unwrap();
                assert_eq!(item.message, message, "split {split}");
                assert_eq!(
                    item.data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
                assert_eq!(
                    item.range,
                    Text::new(source.clone()).range(11, 11 + name.len())
                );
            }
        }
    }

    #[test]
    fn destination_wraps_preserve_real_whitespace_and_incomplete_message_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "\\hyperlink{section.target}{Text}\n").unwrap();
        for message in [
            "pdfTeX warning (dest): name{section. target} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section. target'",
        ] {
            let split = message.find(" target").unwrap();
            let log = format!("{}\n{}\n</fonts/resource.otf>\n", &message[..split], &message[split..]);
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert!(items[0].code.is_none());
            assert!(items[0].data.is_none());
        }
        for boundary in [
            "</fonts/resource.otf>",
            "LaTeX Warning: Next warning.",
            "! Next error.",
            crate::process::output::TRUNCATED,
            "(./main.tex",
        ] {
            let log = format!("pdfTeX warning (dest): name{{section.\n{boundary}\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let item = items
                .iter()
                .find(|item| item.message.starts_with("pdfTeX warning"))
                .unwrap();
            assert_eq!(item.message, "pdfTeX warning (dest): name{section.");
            assert!(item.code.is_none());
        }
    }

    #[test]
    fn unicode_destinations_survive_log_breaks_inside_utf8_characters() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}终点", "章节".repeat(20));
        std::fs::write(&main, format!("\\hyperlink{{{name}}}{{Text}}\n")).unwrap();
        let message = format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist, replaced by a fixed one");
        for split in "pdfTeX warning (dest): name{".len()..message.len() {
            let log = [
                &message.as_bytes()[..split],
                b"\n",
                &message.as_bytes()[split..],
                b"\n</fonts/resource.pfb>\n",
            ]
            .concat();
            let diagnostics = parse(&main, &decode_output(&log));
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "split {split}: {items:?}");
            assert_eq!(
                items[0].data,
                Some(serde_json::json!({"destination":name})),
                "split {split}: {items:?}"
            );
        }
    }

    #[test]
    fn lua_destination_apostrophes_at_print_width_do_not_target_a_different_shorter_name() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let prefix = "warning  (pdf backend): unreferenced destination with name '";
        for width in [79, 80] {
            let shorter = "x".repeat(width - prefix.len() - 1);
            let name = format!("{shorter}'suffix");
            std::fs::write(
                &main,
                format!("\\hyperlink{{{name}}}{{Long}}\\hyperlink{{{shorter}}}{{Short}}\n"),
            )
            .unwrap();
            let message = format!("{prefix}{name}'");
            let log = format!(
                "{}\n{}\n</fonts/resource.otf>\n",
                &message[..width],
                &message[width..]
            );
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert_eq!(items[0].data, Some(serde_json::json!({"destination":name})));
        }
    }

    #[test]
    fn destination_warnings_locate_direct_and_compound_links_without_inventing_lines() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hyperlink{section.lost}{Text}\\label{section.lost}\r\n\\hyperref{}{section}% keep\r\n{lost}{Link}\r\n\\hyperref{remote}{section}{lost}{Remote}\n\\verb|\\hyperlink{section.lost}{Ignored}|\n";
        std::fs::write(&main, source).unwrap();
        let text = Text::new(source.into());
        let first = source.find("section.lost").unwrap();
        let second = source.find("{section}").unwrap() + 1;
        let last = source.find("{lost}").unwrap() + 5;
        for warning in [
            "pdfTeX warning (dest): name{section.lost} has been referenced but does not exis\nt, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section.lost'",
        ] {
            let log = format!("(./main.tex\n{warning}\n<font/resource.otf>\n! Other error.\nl.1 bad\n)\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 3, "{items:?}");
            let references: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("undefined-pdf-destination".into()))).collect();
            assert_eq!(references.len(), 2, "{items:?}");
            assert_eq!(references[0].range, text.range(first, first + 12));
            assert_eq!(references[1].range, text.range(second, last));
            assert!(references.iter().all(|item| item.data == Some(serde_json::json!({"destination":"section.lost"})) && !item.message.contains("font/")));
        }
    }

    #[test]
    fn backend_destination_string_escapes_map_exact_keys_across_all_byte_wraps() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for name in ["dest(old)", "dest(old", "dest)old"] {
            let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            let encoded = name.replace('(', "\\(").replace(')', "\\)");
            for (prefix, suffix, code, offset) in [
                (
                    "pdfTeX warning (dest): name{",
                    "} has been referenced but does not exist, replaced by a fixed one",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "pdfTeX warning (ext4): destination with the same identifier (name{",
                    "}) has been already used, duplicate ignored",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): unreferenced destination with name '",
                    "'",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): ignoring duplicate destination with the name '",
                    "'",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
            ] {
                let message = format!("{prefix}{encoded}{suffix}");
                assert_eq!(
                    destination_warning(&message).map(|(_, key)| key),
                    Some(name.to_owned())
                );
                for split in prefix.len()..message.len() {
                    let log = format!(
                        "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n",
                        &message[..split],
                        &message[split..]
                    );
                    let diagnostics = parse(&main, &log);
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2, "{name}: split {split}: {items:?}");
                    let mapped: Vec<_> = items
                        .iter()
                        .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                        .collect();
                    assert_eq!(mapped.len(), 1, "{name}: split {split}: {items:?}");
                    assert_eq!(mapped[0].range, text.range(offset, offset + name.len()));
                    assert_eq!(
                        mapped[0].data,
                        Some(serde_json::json!({"destination":name}))
                    );
                    assert_eq!(mapped[0].message, message);
                }
            }
        }
        for encoded in [r"dest\", r"dest\dynamic", r"dest\\(old)", r"dest\( old\)"] {
            for message in [
                format!("pdfTeX warning (dest): name{{{encoded}}} has been referenced but does not exist, replaced by a fixed one"),
                format!("pdfTeX warning (ext4): destination with the same identifier (name{{{encoded}}}) has been already used, duplicate ignored"),
                format!("warning  (pdf backend): unreferenced destination with name '{encoded}'"),
                format!("warning  (pdf backend): ignoring duplicate destination with the name '{encoded}'"),
            ] {
                assert!(destination_warning(&message).is_none(), "{message}");
            }
        }
    }

    #[test]
    fn backend_octal_destination_bytes_preserve_unicode_and_reject_invalid_encodings() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for (name, encoded) in [
            (
                "章节.目标",
                r"\347\253\240\350\212\202.\347\233\256\346\240\207",
            ),
            ("emoji😀", r"emoji\360\237\230\200"),
            (
                "mix(目标😀)",
                r"mix\(\347\233\256\346\240\207\360\237\230\200\)",
            ),
            ("dest(old)", r"dest\050old\051"),
        ] {
            let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            for (prefix, suffix, code, offset) in [
                (
                    "pdfTeX warning (dest): name{",
                    "} has been referenced but does not exist, replaced by a fixed one",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "pdfTeX warning (ext4): destination with the same identifier (name{",
                    "}) has been already used, duplicate ignored",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): unreferenced destination with name '",
                    "'",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): ignoring duplicate destination with the name '",
                    "'",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
            ] {
                let message = format!("{prefix}{encoded}{suffix}");
                assert_eq!(
                    destination_warning(&message).map(|(_, key)| key),
                    Some(name.to_owned())
                );
                for split in prefix.len()..message.len() {
                    let log = format!(
                        "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n",
                        &message[..split],
                        &message[split..]
                    );
                    let diagnostics = parse(&main, &log);
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2, "{name}: split {split}: {items:?}");
                    let mapped: Vec<_> = items
                        .iter()
                        .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                        .collect();
                    assert_eq!(mapped.len(), 1, "{name}: split {split}: {items:?}");
                    assert_eq!(mapped[0].range, text.range(offset, offset + name.len()));
                    assert_eq!(
                        mapped[0].data,
                        Some(serde_json::json!({"destination":name}))
                    );
                    assert_eq!(mapped[0].message, message);
                }
            }
        }
        for encoded in [
            r"dest\4",
            r"dest\40",
            r"dest\400",
            r"dest\777",
            r"dest\08a",
            r"dest\999",
            r"dest\300\257",
            r"dest\355\240\200",
            r"dest\377",
            r"dest\000",
            r"dest\012",
            r"dest\177",
            r"dest\302\237",
            r"dest\134",
            r"dest\173",
            r"dest\175",
            r"dest\045",
            r"dest\043",
            r"dest\040",
        ] {
            for message in [
                format!("pdfTeX warning (dest): name{{{encoded}}} has been referenced but does not exist, replaced by a fixed one"),
                format!("pdfTeX warning (ext4): destination with the same identifier (name{{{encoded}}}) has been already used, duplicate ignored"),
                format!("warning  (pdf backend): unreferenced destination with name '{encoded}'"),
                format!("warning  (pdf backend): ignoring duplicate destination with the name '{encoded}'"),
            ] {
                assert!(destination_warning(&message).is_none(), "{message}");
            }
        }
    }

    #[test]
    fn raw_destination_parentheses_at_print_width_keep_complete_names_and_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for (prefix, code, declaration) in [
            (
                "warning  (pdf backend): unreferenced destination with name '",
                "undefined-pdf-destination",
                false,
            ),
            (
                "warning  (pdf backend): ignoring duplicate destination with the name '",
                "duplicate-pdf-destination",
                true,
            ),
        ] {
            for suffix in ["(paired)", "(unpaired", ")unpaired", "(目标😀)"] {
                let name = format!("{}{suffix}", "x".repeat(80 - prefix.len()));
                let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\r\n");
                std::fs::write(&main, &source).unwrap();
                let text = Text::new(source.clone());
                let at = if declaration {
                    source.find(&name)
                } else {
                    source.rfind(&name)
                }
                .unwrap();
                let message = format!("{prefix}{name}'");
                let mut log = String::new();
                let mut start = 0;
                while start < message.len() {
                    let mut end = (start + 80).min(message.len());
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    log.push_str(&message[start..end]);
                    log.push('\n');
                    start = end;
                }
                log.push_str("</fonts/resource.otf>\nLaTeX Warning: Keep.\n");
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "{message}: {items:?}");
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                    .collect();
                assert_eq!(mapped.len(), 1, "{message}: {items:?}");
                assert_eq!(mapped[0].range, text.range(at, at + name.len()));
                assert_eq!(
                    mapped[0].data,
                    Some(serde_json::json!({"destination":name}))
                );
                assert_eq!(mapped[0].message, message);
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
            }
        }
    }

    #[test]
    fn incomplete_raw_destination_wraps_preserve_file_stack_and_following_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("child.tex");
        std::fs::write(&main, "Main source\n").unwrap();
        std::fs::write(&child, "Child source\n").unwrap();
        for prefix in [
            "warning  (pdf backend): unreferenced destination with name '",
            "warning  (pdf backend): ignoring duplicate destination with the name '",
        ] {
            let partial = format!("{prefix}{}", "x".repeat(80 - prefix.len()));
            for log in [
                format!("{partial}\n(./child.tex\nLaTeX Warning: Child sentinel.\n)\nLaTeX Warning: Main sentinel.\n"),
                format!("(./child.tex\n{partial}\n)\nLaTeX Warning: Main sentinel.\n"),
                format!("{partial}\n</fonts/resource.otf>\nchild.tex:1: LaTeX Error: Child sentinel.\nLaTeX Warning: Main sentinel.\n"),
            ] {
                let diagnostics = parse(&main, &log);
                let all: Vec<_> = diagnostics.values().flatten().collect();
                assert!(all.iter().all(|item| item.code.is_none() && item.data.is_none()), "{log}: {diagnostics:?}");
                assert_eq!(all.iter().filter(|item| item.message == partial).count(), 1, "{log}: {diagnostics:?}");
                let main_items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert!(main_items.iter().any(|item| item.message == "LaTeX Warning: Main sentinel."), "{log}: {diagnostics:?}");
                let child_items = &diagnostics[&Url::from_file_path(&child).unwrap()];
                assert!(child_items.iter().any(|item| item.message.contains("Child sentinel.") || item.message == partial), "{log}: {diagnostics:?}");
            }
        }
    }

    #[test]
    fn lua_unicode_early_wraps_do_not_diagnose_a_healthy_short_prefix() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let missing = "warning  (pdf backend): unreferenced destination with name '";
        let duplicate = "warning  (pdf backend): ignoring duplicate destination with the name '";
        // Widths observed in successful native LuaTeX compilations, where the
        // next whole Unicode character no longer fits on the physical line.
        let cases: [(&str, char, &[usize], bool); 6] = [
            (missing, 'é', &[77, 78], false),
            (missing, '目', &[76, 77, 78], false),
            (missing, '😀', &[75, 76, 77, 78], false),
            (duplicate, 'é', &[76, 77, 78], true),
            (duplicate, '目', &[75, 76, 77, 78], true),
            (duplicate, '😀', &[75, 76, 77, 78], true),
        ];
        for (prefix, character, widths, declaration) in cases {
            for width in widths {
                let short = "x".repeat(width - prefix.len() - 1);
                let name = format!("{short}'{character}tail");
                let source = if declaration {
                    format!("😀 \\hypertarget{{{short}}}{{Healthy}}\\hypertarget{{{name}}}{{One}}\\hypertarget{{{name}}}{{Two}}\\label{{{name}}}")
                } else {
                    format!("😀 \\hypertarget{{{short}}}{{Healthy}}\\hyperlink{{{short}}}{{Healthy}}\\hyperlink{{{name}}}{{Missing}}\\label{{{name}}}")
                };
                std::fs::write(&main, &source).unwrap();
                let text = Text::new(source.clone());
                let message = format!("{prefix}{name}'");
                let log = format!(
                "{prefix}{short}'\n{character}tail'\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n"
            );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                let code = if declaration {
                    "duplicate-pdf-destination"
                } else {
                    "undefined-pdf-destination"
                };
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                    .collect();
                let count = if declaration { 2 } else { 1 };
                assert_eq!(items.len(), count + 1, "{message}: {items:?}");
                assert_eq!(mapped.len(), count, "{message}: {items:?}");
                for (item, (at, _)) in mapped.iter().zip(source.match_indices(&name).take(count)) {
                    assert_eq!(item.range, text.range(at, at + name.len()));
                    assert_eq!(item.data, Some(serde_json::json!({"destination":name})));
                    assert_eq!(item.message, message);
                }
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
            }
        }
    }

    #[test]
    fn unicode_lookahead_after_a_complete_destination_keeps_independent_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("文件.tex");
        std::fs::write(&child, "Child source\n").unwrap();
        let prefix = "warning  (pdf backend): unreferenced destination with name '";
        for width in [74, 75, 78] {
            let name = "x".repeat(width - prefix.len() - 1);
            let source = format!("😀 \\hyperlink{{{name}}}{{Link}}\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            let at = source.find(&name).unwrap();
            let message = format!("{prefix}{name}'");
            for (next, child_error) in [
                ("文件.tex:1: LaTeX Error: Unicode sentinel.", true),
                ("正文 without a closing apostrophe", false),
            ] {
                let log = format!("{message}\n{next}\nLaTeX Warning: Keep.\n");
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "{log}: {diagnostics:?}");
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| {
                        item.code
                            == Some(NumberOrString::String("undefined-pdf-destination".into()))
                    })
                    .collect();
                assert_eq!(mapped.len(), 1, "{log}: {diagnostics:?}");
                assert_eq!(mapped[0].range, text.range(at, at + name.len()));
                assert_eq!(mapped[0].message, message);
                assert_eq!(
                    mapped[0].data,
                    Some(serde_json::json!({"destination":name}))
                );
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
                let children = diagnostics
                    .get(&Url::from_file_path(&child).unwrap())
                    .map_or(&[][..], Vec::as_slice);
                assert_eq!(
                    children.len(),
                    usize::from(child_error),
                    "{log}: {diagnostics:?}"
                );
                if child_error {
                    assert_eq!(children[0].message, "LaTeX Error: Unicode sentinel.");
                }
            }
        }
    }

    #[test]
    fn destination_warning_names_preserve_semantic_spaces_and_reject_uncertain_messages() {
        for name in [
            "lost",
            "section.foo",
            "章节.目标",
            "dest(old)",
            "dest(old",
            "dest)old",
        ] {
            for message in [format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist , replaced by a fixed one"), format!("warning  (pdf backend): unreferenced destination with name '{name}'")] {
                assert_eq!(undefined_destination_name(&message).as_deref(), Some(name));
            }
        }
        for message in [
            "pdfTeX warning (dest): name{a b} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name ' lost'",
            "warning  (pdf backend): unreferenced destination with name 'lost ' ",
            "pdfTeX warning (dest): name{lost} has been referenced",
            "warning  (pdf backend): unreferenced destination with name '\\dynamic'",
            "pdfTeX warning (dest): name{lost} has been referenced but was found",
        ] {
            assert_eq!(undefined_destination_name(message), None, "{message}");
        }
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First line\n\\hyperlink{\\dynamic}{Text}\n").unwrap();
        let diagnostics = parse(&main, "warning  (pdf backend): unreferenced destination with name 'unknown'\n<font/resource.otf>\n");
        let items = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none());
        assert!(items[0].data.is_none());
        assert!(!items[0].message.contains("font/"));
    }

    #[test]
    fn final_pdf_page_counts_support_engine_variants_and_wrapped_summaries() {
        for (log, expected) in [
            ("Output written on main.pdf (1 page, 123 bytes).\n", Some(1)),
            ("Output written on main.pdf (12 pages).\n", Some(12)),
            ("Output written on /long/path (draft)/\nmain.pdf (2 pa\nges, 3456 bytes).\n", Some(2)),
            ("Output written on main.pdf (3 pages, 123 bytes).\nOutput written on main.pdf (4 pages, 456 bytes).\n", Some(4)),
            ("Writing main.pdf\n", None),
            ("Output written on main.pdf (0 pages, 0 bytes).\n", None),
            ("Output written on main.pdf (999999999999 pages, 9 bytes).\n", None),
            ("Output written on main.pdf (2 pages, invalid bytes).\n", None),
        ] {
            assert_eq!(page_count(log), expected, "{log}");
        }
    }

    #[test]
    fn tex_byte_wrapping_recovers_split_utf8_without_joining_valid_or_malformed_lines() {
        for scalar in ["é", "中", "😀"] {
            for split in 1..scalar.len() {
                for newline in [b"\n".as_slice(), b"\r\n".as_slice()] {
                    let bytes = [
                        b"No file dir/",
                        &scalar.as_bytes()[..split],
                        newline,
                        &scalar.as_bytes()[split..],
                        b".tex.\n",
                    ]
                    .concat();
                    assert_eq!(
                        decode_output(&bytes),
                        format!("No file dir/{scalar}.tex.\n")
                    );
                }
            }
        }
        assert_eq!(decode_output("中文\n😀\n".as_bytes()), "中文\n😀\n");
        for bytes in [
            b"\xff\n\x80".as_slice(),
            b"\xe0\x80\n\x80".as_slice(),
            b"\xe4\nA".as_slice(),
            b"\xe4\n\n\xb8\xad".as_slice(),
        ] {
            assert_eq!(decode_output(bytes), String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_follow_source_aliases_with_unrelated_target_extensions() {
        for extension in ["payload", "png", "tex", "bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let database = root.join(format!("database.{extension}"));
            let style = root.join(format!("style.{extension}"));
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(
                &database,
                "% database\n@book{entry,title={Book}}\n@book{entry,title={Duplicate}}\n@book{unique,title={Warning}}\n",
            )
            .unwrap();
            std::fs::write(&style, "ENTRY {title} {} {}\nFUNCTION {broken} {unknown}\n").unwrap();
            std::os::unix::fs::symlink(&database, root.join("with spaces.bib")).unwrap();
            std::os::unix::fs::symlink(&style, root.join("local.bst")).unwrap();
            let diagnostics = parse_bibliography(&main, "Database file #1: with spaces.bib\nThe style file: local.bst\nRepeated entry---line 3 of file \"with spaces.bib\"\nunknown is an unknown function---line 2 of file local.bst\nWarning--empty author in unique\n");
            let database_uri = Url::from_file_path(&database).unwrap();
            let style_uri = Url::from_file_path(&style).unwrap();
            assert_eq!(diagnostics.len(), 2, "{extension}: {diagnostics:?}");
            let errors = &diagnostics[&database_uri];
            assert_eq!(errors.len(), 2);
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: Repeated entry"
                    && item.range.start.line == 2
                    && item.severity == Some(DiagnosticSeverity::ERROR)));
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: empty author in unique"
                    && item.range.start == Position::new(3, 6)
                    && item.severity == Some(DiagnosticSeverity::WARNING)));
            assert_eq!(diagnostics[&style_uri][0].range.start.line, 1);
            assert_eq!(
                diagnostics[&style_uri][0].message,
                "BibTeX: unknown is an unknown function"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_do_not_treat_auxiliary_alias_targets_as_sources() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let physical = root.join("physical.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&physical, "% database\n@book{entry,\n title={Book}}\n").unwrap();
        for name in ["main.aux", "raw.csv"] {
            std::os::unix::fs::symlink(&physical, root.join(name)).unwrap();
        }
        std::fs::create_dir(root.join("directory.bst")).unwrap();
        for name in ["main.aux", "raw.csv", "missing.bib", "directory.bst"] {
            let log = format!("Error in auxiliary input---line 3 of file {name}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start.line, 0);
            assert_eq!(items[0].message, "BibTeX: Error in auxiliary input");
        }
    }

    #[test]
    fn bibliography_diagnostics_use_the_tool_working_directory_for_database_and_style_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        for name in ["refs.bib", "build/refs.bib"] {
            std::fs::write(
                root.join(name),
                "% database\n@book{example,title={Example}}\n",
            )
            .unwrap();
        }
        for name in ["local.bst", "build/local.bst"] {
            std::fs::write(
                root.join(name),
                "ENTRY {title} {} {}\nFUNCTION {bad} {unknown}\n",
            )
            .unwrap();
        }
        for (auxiliary, prefix) in [("main.aux", "build/"), ("build/main.aux", "")] {
            let log = format!("The top-level auxiliary file: {auxiliary}\nDatabase file #1: refs.bib\nThe style file: local.bst\nWarning--empty author in example\nRepeated entry---line 2 of file refs.bib\nunknown is an unknown function---line 2 of file local.bst\n");
            let diagnostics =
                parse_bibliography_log(&root.join("main.tex"), &root.join("build/main.blg"), &log);
            let bib = Url::from_file_path(root.join(format!("{prefix}refs.bib"))).unwrap();
            let style = Url::from_file_path(root.join(format!("{prefix}local.bst"))).unwrap();
            assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
            assert_eq!(diagnostics[&bib].len(), 2);
            assert!(diagnostics[&bib]
                .iter()
                .all(|diagnostic| diagnostic.range.start.line == 1));
            assert_eq!(diagnostics[&style][0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_diagnostic_paths_do_not_resolve_prefixes_of_longer_filenames() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{entry}\nText.\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% decoy\n@book{decoy,title={Wrong}}\n",
        )
        .unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        for name in [
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib-archive",
            "refs.bib_archive",
            "refs.bib中文",
            "refs.bib/nonexistent.bib",
        ] {
            if !name.contains('/') {
                std::fs::write(
                    root.join(name),
                    "% actual input\n@book{entry,title={Book}}\n",
                )
                .unwrap();
            }
            for quoted in [false, true] {
                let name = if quoted {
                    format!("'{name}'")
                } else {
                    name.into()
                };
                let message = format!("syntax error in file {name}, line 2");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
                let items = diagnostics
                    .get(&main_uri)
                    .unwrap_or_else(|| panic!("{name}: {diagnostics:?}"));
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_diagnostic_paths_keep_complete_names_before_message_punctuation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% database\n@book{entry,title={Book}}\n").unwrap();
        for message in [
            "syntax error in file 'refs.bib', line 2",
            "syntax error in file \"refs.bib\", line 2",
            "syntax error in refs.bib, line 2",
            "refs.bib: syntax error at line 2",
            "syntax error in refs.bib; line 2",
            "syntax error (refs.bib) at line 2",
            "syntax error [refs.bib] at line 2",
            "syntax error in refs.bib. See line 2",
            "syntax error on line 2 of refs.bib.",
            "syntax error on line 2 of refs.bib",
        ] {
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].message, format!("Biber: {message}"));
            assert_eq!(items[0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_quoted_paths_keep_spaces_apostrophes_and_internal_bib_suffixes() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let target = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&target, "% database\n\n@book{entry,title={Correct}}\n").unwrap();
            for quote in ['\'', '"'] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - Error in {quote}{name}{quote} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\nWARN - Missing another field for entry 'entry' in file {quote}{name}{quote}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{name} {quote}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
                assert_eq!(items.len(), 3);
                assert!(items.iter().all(|item| item.range.start.line == 2));
                for warning in items
                    .iter()
                    .filter(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                {
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_explicit_paths_preserve_unreported_files_and_do_not_replace_changed_inputs() {
        for name in ["plain.bib", "refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let blg = root.join("main.blg");
            let explicit = root.join(name);
            let input = selected.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Read input}}\n").unwrap();
            for source in [explicit.to_str().unwrap().to_owned(), format!("../{name}")] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file '{source}', line 3\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                let paths = crate::compilation::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let items = diagnostics
                    .get(&Url::from_file_path(&explicit).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(items.len(), 2);
                assert!(
                    items
                        .iter()
                        .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                    "{log}: {items:?}"
                );
                assert_eq!(
                    crate::compilation::recorder::bibliography_inputs_in(&paths, &log),
                    [Url::from_file_path(&input).unwrap()].into()
                );
            }
            let log = format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' (file '{name}')\n");
            let paths = crate::compilation::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_share_absolute_relative_and_dot_spellings() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("inputs")).unwrap();
        std::fs::create_dir(root.join("build")).unwrap();
        let main = root.join("main.tex");
        let blg = root.join("build/main.blg");
        let name = "inputs/中文 refs.bib': in file 'wrong.bib' line 700 copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
        for decoy in ["inputs/中文 refs.bib", "wrong.bib", "copy.bib"] {
            std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
        }
        let spellings = [
            name.to_owned(),
            format!("./{name}"),
            bib.to_str().unwrap().to_owned(),
            root.join(format!("./{name}")).to_str().unwrap().to_owned(),
        ];
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&bib).unwrap();
            }
            for input in &spellings {
                for source in &spellings {
                    for quote in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {quote}{source}{quote}, line 3"),
                            format!("syntax error {quote}{source}{quote}, line 3"),
                            format!("Missing field {quote}{source}{quote} for entry 'entry'"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{input}'\n{} - {message}\n",
                                if message.starts_with("Missing") {
                                    "WARN"
                                } else {
                                    "ERROR"
                                }
                            );
                            let diagnostics = parse_bibliography_log(&main, &blg, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            assert_eq!(items.len(), 1);
                            if missing {
                                assert_eq!(items[0].range.start.line, 0);
                                assert_eq!(
                                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                                    true
                                );
                            } else {
                                assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                if message.starts_with("Missing") {
                                    assert_eq!(
                                        items[0].range,
                                        Range::new(Position::new(2, 6), Position::new(2, 11))
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_respect_search_paths_and_selected_working_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let output = root.join("build");
        std::fs::create_dir(&output).unwrap();
        let main = root.join("main.tex");
        let blg = output.join("main.blg");
        let name = "refs.bib': line 700 copy.bib";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        for base in [root, &output] {
            std::fs::write(base.join(name), "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(base.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        }
        for auxiliary in ["main.aux", "build/main.aux"] {
            for selected in [root, &output] {
                for absolute_input in [false, true] {
                    let input = if absolute_input {
                        selected.join(name).to_str().unwrap().to_owned()
                    } else {
                        name.to_owned()
                    };
                    let source = if absolute_input {
                        name.to_owned()
                    } else {
                        selected.join(name).to_str().unwrap().to_owned()
                    };
                    let log = format!("The top-level auxiliary file: {auxiliary}\nINFO - Found BibTeX data source '{input}'\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                    let paths =
                        crate::compilation::recorder::BibliographyPaths::new(&main, &blg, &log)
                            .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                    let diagnostics = parse_bibliography_in(&main, &paths, &log);
                    let target = Url::from_file_path(selected.join(name)).unwrap();
                    let items = diagnostics
                        .get(&target)
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
        // An absolute input record disambiguates an unreported full-name
        // lookalike in cwd, even if cwd precedes the log directory.
        let actual = output.join(name);
        let log = format!("INFO - Found BibTeX data source '{}'\nWARN - Missing field for entry 'entry' (file '{name}')\n", actual.display());
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&actual).unwrap();
            }
            for search in [String::new(), format!("BIBINPUTS='{}'\n", root.display())] {
                let paths = crate::compilation::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&search);
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let target = if missing { &main } else { &actual };
                let items = &diagnostics[&Url::from_file_path(target).unwrap()];
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                assert_eq!(items.len(), 1);
                if missing {
                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn repeated_bibliography_records_do_not_hide_a_later_complete_source_name() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "refs.bib': copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        let mut log = "INFO - Found BibTeX data source 'refs.bib'\n".repeat(4096);
        log.push_str(&format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' in file '{name}'\n"));
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let items = diagnostics
            .get(&Url::from_file_path(&bib).unwrap())
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
    }

    #[test]
    #[cfg(unix)]
    fn biber_logged_quoted_names_disambiguate_punctuation_spaces_and_metadata() {
        for suffix in [
            ": copy.bib",
            "; copy.bib",
            ") copy.bib",
            "] copy.bib",
            "} copy.bib",
            ", copy.bib",
            ": line 700 copy.bib",
            ", in file 'wrong.bib' copy.bib",
            ", entry 'wrong' copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let bib = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib", "wrong.bib"] {
                    std::fs::write(
                        root.join(decoy),
                        "@book{entry,title={Wrong}}\n@book{wrong}\n",
                    )
                    .unwrap();
                }
                std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for missing in [false, true] {
                    if missing {
                        std::fs::remove_file(&bib).unwrap();
                    }
                    for outer in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {outer}{name}{outer}, line 3"),
                            format!("syntax error on line 3 in file {outer}{name}{outer}"),
                            format!("syntax error {outer}{name}{outer}: line 3"),
                            format!("Missing field for entry 'entry' (file {outer}{name}{outer})"),
                            format!("Missing field {outer}{name}{outer} for entry 'entry'"),
                        ] {
                            // Both names are reported; their order and existence
                            // cannot make a literal long name select the short one.
                            for names in [
                                format!("INFO - Found BibTeX data source '中文 refs.bib'\nINFO - Found BibTeX data source '{name}'\n"),
                                format!("INFO - Found BibTeX data source '{name}'\nINFO - Found BibTeX data source '中文 refs.bib'\n"),
                            ] {
                                let log = format!("{names}{} - {message}\n", if message.starts_with("Missing") { "WARN" } else { "ERROR" });
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap()).unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                if missing {
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true, "{log}: {items:?}");
                                    assert_eq!(items[0].range.start.line, 0);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)), "{log}: {items:?}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_paths_keep_punctuation_and_line_text_after_literal_quotes() {
        for suffix in [
            ":copy.bib",
            ";copy.bib",
            ")copy.bib",
            "]copy.bib",
            "}copy.bib",
            ",copy.bib",
            "),;copy.bib",
            " line 700 copy.bib",
            " line 700.copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let target = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib"] {
                    std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
                }
                std::fs::write(&target, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 3"),
                    format!("syntax error on line 3 in file {quote}{name}{quote}"),
                    format!("syntax error {quote}{name}{quote}: line 3"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let items = diagnostics
                        .get(&Url::from_file_path(&target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                        "{log}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
                std::fs::remove_file(&target).unwrap();
                let log = format!("INFO - Found BibTeX data source '中文 refs.bib'\nERROR - syntax error in file {quote}{name}{quote}, line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2);
                assert!(
                    items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)),
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_missing_quoted_paths_do_not_borrow_prefix_suffix_or_other_database_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let source = "\\cite{entry}\n";
        std::fs::write(&main, source).unwrap();
        for name in ["refs.bib", "copy.bib", "unrelated.bib"] {
            std::fs::write(root.join(name), "% decoy\n\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for quote in ['\'', '"'] {
            for name in ["refs.bib missing copy.bib", "directory.bib"] {
                for closed in [false, true] {
                    let closing = if closed {
                        quote.to_string()
                    } else {
                        String::new()
                    };
                    let log = format!("INFO - Found BibTeX data source 'unrelated.bib'\nERROR - Error in {quote}{name}{closing} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{closing})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{quote}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2);
                    let text = Text::new(source.into());
                    assert!(items
                        .iter()
                        .all(|item| item.range.start.line == 0
                            && text.offset(item.range.end).is_some()));
                    assert!(items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)));
                }
            }
        }
    }

    #[test]
    fn biber_explicit_file_warnings_never_reuse_another_databases_entry_range() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let target = root.join("actual.bib");
        let contents = "@book{other,title={Actual}}\n";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&target, contents).unwrap();
        std::fs::write(
            root.join("unrelated.bib"),
            "% other\n\n\n@book{entry,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'unrelated.bib'\nWARN - Missing field for entry 'entry' (file 'actual.bib')\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
        assert!(Text::new(contents.into())
            .offset(items[0].range.end)
            .is_some());
    }

    #[test]
    fn bibtex_entry_keys_are_not_interpreted_as_namesake_source_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{'entry'}\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{wrong,title={Decoy}}\n").unwrap();
        for key in ["'entry'", "'entry.bib'", "entry.bib"] {
            std::fs::write(&bib, format!("@book{{{key},title={{Book}}}}\n")).unwrap();
            let log =
                format!("Database file #1: refs.bib\nWarning--empty author and editor in {key}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{key}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].range,
                Range::new(
                    Position::new(0, 6),
                    Position::new(0, 6 + key.encode_utf16().count() as u32)
                )
            );
        }
    }

    #[test]
    fn biber_line_fields_ignore_complete_quoted_source_names() {
        for name in [
            "line 700 refs.bib",
            "outline 700 refs.bib",
            "line 700 author's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
            for quote in ['\'', '"'] {
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 2"),
                    format!("syntax error on line 2 in file {quote}{name}{quote}"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 1 && !unresolved_location(item)),
                        "{message}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(1, 6), Position::new(1, 11))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_line_fields_require_unquoted_complete_words_and_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
        for prefix in [
            "outline 700; error",
            "pipeline 700; error",
            "_line 700; error",
            "unexpected token 'line 700'",
            "unexpected token \"line 700\"",
            "unexpected token 'author's line 700'",
            "invalid line 700bad; error",
            "invalid line 700.5; error",
        ] {
            let message = format!("{prefix} in file 'refs.bib', line 2");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].range.start.line, 1, "{message}: {items:?}");
            assert!(!unresolved_location(&items[0]), "{message}: {items:?}");
        }
        for number in [0, u32::MAX] {
            let message = format!("syntax error in file 'refs.bib', line {number}");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["reportedLine"], number);
            assert!(unresolved_location(&items[0]));
        }
    }

    #[test]
    fn bibtex_warning_text_does_not_override_the_entry_location_with_a_line_number() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n").unwrap();
        let diagnostics = parse_bibliography(
            &main,
            "Database file #1: refs.bib\nWarning--line 700 value in entry\n",
        );
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
        assert!(!unresolved_location(&items[0]));
    }

    #[test]
    fn biber_entry_fields_preserve_internal_leading_and_trailing_quotes() {
        for key in [
            "O'Connor2026",
            "'start",
            "end'",
            "two''quotes",
            "name'-part",
            "name'_part",
            "name'.part",
            "name':part",
            "name';part",
            "name']part",
            "name'!part",
            "name'?part",
            "王'文2026",
            "😀'key",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            // Prefix and suffix declarations must not replace the full key.
            std::fs::write(&bib, format!("@book{{O,title={{Prefix}}}}\n@book{{Connor2026,title={{Suffix}}}}\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    let message = format!("Missing required field for {label} {quote}{key}{quote}");
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(
                        items[0].range,
                        Range::new(
                            Position::new(2, 6),
                            Position::new(2, 6 + key.encode_utf16().count() as u32)
                        ),
                        "{message}: {items:?}"
                    );
                    assert_eq!(items[0].message, format!("Biber: {message}"));
                }
            }
        }
    }

    #[test]
    fn biber_entry_keys_do_not_select_namesake_database_files() {
        for key in ["entry.bib", "'entry.bib'", "O'Connor.bib", "dir/entry.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            let decoy = root.join(key);
            std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, format!("% header\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            std::fs::write(
                &decoy,
                format!("% decoy\n\n\n@book{{{key},title={{Wrong}}}}\n"),
            )
            .unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    for source in ["", " in refs.bib", " in file 'refs.bib'"] {
                        let log = format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for {label} {quote}{key}{quote}{source}\n");
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range,
                            Range::new(
                                Position::new(1, 6),
                                Position::new(1, 6 + key.encode_utf16().count() as u32)
                            ),
                            "{log}: {items:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_logged_unquoted_sources_keep_complete_names_outside_message_fields() {
        for name in [
            "refs.bib line 700 copy.bib",
            "中文 refs.bib in file 'wrong.bib' entry 'wrong' copy.bib",
            "refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for input in [name.to_owned(), bib.to_str().unwrap().to_owned()] {
                    for source in [
                        name.to_owned(),
                        format!("./{name}"),
                        bib.to_str().unwrap().to_owned(),
                    ] {
                        for (open, close) in [("", ""), ("(", ")"), ("[({", "})]")] {
                            for message in [
                                format!("Unexpected token \"in 'wrong.bib', line 900\" in file {open}{source}{close}, line 3"),
                                format!("syntax error {open}{source}{close}, line 3"),
                                format!("Missing field {open}{source}{close} for entry 'entry'"),
                                format!("Missing field for entry 'entry' (file {open}{source}{close})"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{input}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics
                                    .get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                if missing {
                                    assert_eq!(items[0].range.start, Position::new(0, 0));
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_unquoted_prefixes_do_not_shorten_complete_unreported_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for name in ["refs.bib,copy.bib", "refs.bib:copy.bib"] {
            let bib = root.join(name);
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for message in [
                    format!("syntax error in file {name}, line 3"),
                    format!("Missing field for entry 'entry' (file {name})"),
                ] {
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let target = if missing { &main } else { &bib };
                    let items = diagnostics
                        .get(&Url::from_file_path(target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 });
                    assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unquoted_sources_preserve_explicit_files_and_missing_selected_inputs() {
        for name in ["plain.bib", "refs.bib line 700 copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let explicit = root.join(name);
            let input = selected.join(name);
            let blg = root.join("main.blg");
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Selected}}\n").unwrap();
            let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file {}, line 3\nWARN - Missing field for entry 'entry' (file {})\n", explicit.display(), explicit.display());
            let paths = crate::compilation::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&explicit).unwrap()];
            assert_eq!(items.len(), 2);
            assert!(
                items
                    .iter()
                    .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                "{log}: {items:?}"
            );
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(
                &main,
                &paths,
                &format!("WARN - Missing field for entry 'entry' (file {name})\n"),
            );
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start, Position::new(0, 0));
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unreported_explicit_sources_keep_spaces_and_do_not_borrow_message_files() {
        for name in [
            "missing refs.bib",
            "refs.bib line 700 copy.bib",
            "refs.bib, copy.bib",
            "中文 refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for marker in ["file", "File", "in", "In", "in file", "in \tFile"] {
                    for (open, close) in [("", ""), ("[({", "})]")] {
                        for message in [
                            format!(
                                "Unexpected token 'wrong.bib' {marker} {open}{name}{close}, line 3"
                            ),
                            format!(
                                "Missing field for entry 'entry' ({marker} {open}{name}{close})"
                            ),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1);
                            assert_eq!(items.len(), 1);
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                            assert_eq!(
                                items[0].range.start.line,
                                if missing { 0 } else { 2 },
                                "{log}: {items:?}"
                            );
                            assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                            if !missing && message.starts_with("Missing") {
                                assert_eq!(
                                    items[0].range,
                                    Range::new(Position::new(2, 6), Position::new(2, 11))
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_explicit_multiword_non_bibliography_names_do_not_borrow_short_sources() {
        for name in ["missing refs.bib.backup", "missing refs.bib2"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            std::fs::write(&main, "Text\n").unwrap();
            for file in [name, "refs.bib", "wrong.bib"] {
                std::fs::write(root.join(file), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for message in [
                format!("Unexpected token 'wrong.bib' in file {name}, line 3"),
                format!("Missing field for entry 'entry' (file {name})"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_generic_prose_does_not_hide_the_actual_source_field() {
        for name in ["refs.bib", "中文 refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for prose in ['\'', '"'] {
                    for quote in ['\'', '"'] {
                        for marker in ["file", "in file", "in"] {
                            for message in [
                                format!("Error in {prose}parser{prose}; {marker} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; for entry 'entry' ({marker} {quote}{name}{quote})"),
                                format!("Error in {prose}parser{prose}; {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {quote}{name}{quote} for entry 'entry'"),
                                format!("Error in {prose}parser{prose}; {name}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {name} for entry 'entry'"),
                                format!("Error in {prose}parser{prose} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose} {quote}{name}{quote} for entry 'entry'"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 }, "{log}: {items:?}");
                                assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                                if !missing && message.starts_with("Missing") {
                                    assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_sources_take_precedence_over_quoted_error_filenames() {
        for name in [
            "refs.bib",
            "author's.bib",
            "中文.bib",
            "dir.bib/refs.bib",
            "refs.bib,copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::create_dir_all(bib.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in ["wrong.bib", "s.bib", "copy.bib", "refs.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            for content in [
                "\"in 'wrong.bib', line 900\"",
                "'file \"wrong.bib\"'",
                "\"wrong.bib\"",
                "'wrong.bib'",
            ] {
                for marker in [
                    "in file",
                    "file",
                    "File",
                    "in",
                    "In",
                    "in file ",
                    "in file \u{3000}",
                ] {
                    for message in [
                        format!(
                            "Error in parser: unexpected token {content} {marker} {name}, line 2"
                        ),
                        format!("{marker} {name}, unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} ({name}), line 2"),
                        format!("{marker} [{name}], unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} {{{name}}}, line 2"),
                        format!("Unexpected token {content} {marker} [({name})], line 2"),
                    ] {
                        let log = format!(
                            "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                        );
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range.start,
                            Position::new(1, 0),
                            "{log}: {items:?}"
                        );
                        assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                        assert_eq!(items[0].message, format!("Biber: {message}"));
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_missing_sources_do_not_borrow_quoted_files_or_entry_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "refs.bib"] {
            std::fs::write(root.join(name), "% header\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for name in [
            "missing.bib",
            "directory.bib",
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib's-backup",
            "refs.bib/nonexistent.bib",
            "(refs.bib",
            "[refs.bib",
            "{refs.bib",
        ] {
            for message in [
                format!("Unexpected token \"in 'wrong.bib'\" in file {name}, line 2"),
                format!("Missing field for entry 'entry' in file {name}"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&main).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(
                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                    true,
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_unquoted_bare_sources_ignore_nested_filenames_in_quoted_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "s.bib", "refs.bib", "author's.bib"] {
            std::fs::write(root.join(name), "% header\n@book{entry,title={Actual}}\n").unwrap();
        }
        for name in ["refs.bib", "author's.bib"] {
            for tail in [
                format!("{name}: syntax error at line 2"),
                format!("syntax error ({name}) at line 2"),
                format!("syntax error [{name}] at line 2"),
                format!("syntax error on line 2 of {name}."),
                format!("syntax error \u{3000}{name}, line 2"),
                format!("syntax error \u{a0}{name}, line 2"),
            ] {
                for message in [
                    format!("Unexpected token \"in 'wrong.bib', line 900\"; {tail}"),
                    format!("Unexpected token \"bang\" {tail} \"detail\""),
                    format!("Unexpected token \"in 'wrong.bib', line 900\" {tail} \"detail\""),
                ] {
                    let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = diagnostics
                        .get(&Url::from_file_path(root.join(name)).unwrap())
                        .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                    assert_eq!(
                        items[0].range.start,
                        Position::new(1, 0),
                        "{message}: {items:?}"
                    );
                    assert!(!unresolved_location(&items[0]));
                }
            }
        }
    }

    #[test]
    fn biber_standalone_quoted_sources_preserve_complete_filename_boundaries() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for quote in ['\'', '"'] {
                let message = format!("syntax error {quote}{name}{quote}, line 3");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&bib).unwrap())
                    .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                assert_eq!(
                    items[0].range.start,
                    Position::new(2, 0),
                    "{message}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_source_fields_ignore_markers_in_quoted_error_content() {
        for name in ["refs.bib", "author's 中文 refs.bib", "entry 'wrong'.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for (outer, inner) in [('"', '\''), ('\'', '"')] {
                for marker in ["in", "In", "file", "File"] {
                    let content = format!(
                        "Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer}"
                    );
                    for quote in ['\'', '"'] {
                        let source = format!("in file {quote}{name}{quote}");
                        for message in [
                            format!("{content} {source}, line 2"),
                            format!("{source}, {content}, line 2"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            let items = diagnostics
                                .get(&Url::from_file_path(&bib).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(
                                items[0].range.start,
                                Position::new(1, 0),
                                "{log}: {items:?}"
                            );
                            assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_missing_source_fields_do_not_use_files_named_in_quoted_error_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Other}}\n").unwrap();
        std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for (outer, inner) in [('"', '\''), ('\'', '"')] {
            for marker in ["in", "In", "file", "File"] {
                let message = format!("Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer} in file 'missing.bib', line 2");
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_incomplete_entry_keys_do_not_select_namesake_database_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{entry.bib,title={Wrong}}\n").unwrap();
        for quote in ['\'', '"'] {
            let log = format!("WARN - Missing field for entry {quote}entry.bib\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
            let items = diagnostics
                .get(&Url::from_file_path(&main).unwrap())
                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
            assert_eq!(items[0].range.start, Position::new(0, 0));
        }
    }

    #[test]
    fn biber_entry_fields_skip_names_inside_files_and_find_metadata_after_them() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "entry 'wrong'.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{O'Connor2026}\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file \"{name}\"\nWARN - Missing field in file \"{name}\" for entry 'O'Connor2026'\n");
        let diagnostics = parse_bibliography(&main, &log);
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(items.len(), 2);
        let error = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::ERROR))
            .unwrap();
        assert_eq!(error.range.start, Position::new(0, 0));
        let warning = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(
            warning.range,
            Range::new(Position::new(2, 6), Position::new(2, 18))
        );
    }

    #[test]
    fn biber_unterminated_entry_fields_do_not_borrow_trailing_keys() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{other}\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% header\n@book{other,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for entry 'missing in other\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
    }

    #[test]
    fn biber_entry_fields_ignore_quoted_content_and_partial_words() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        for prefix in [
            "unexpected token \"entry 'wrong'\"",
            "unexpected token 'entry \"wrong\"'",
            "subentry 'wrong'",
            "_entry 'wrong'",
            "王entry 'wrong'",
        ] {
            for suffix in ["", " for Entry 'O'Connor2026': missing author"] {
                let log = format!(
                    "INFO - Found BibTeX data source 'refs.bib'\nWARN - {prefix}{suffix}\n"
                );
                let diagnostics = parse_bibliography(&main, &log);
                let expected = if suffix.is_empty() { &main } else { &bib };
                let items = &diagnostics[&Url::from_file_path(expected).unwrap()];
                assert_eq!(items.len(), 1, "{log}: {items:?}");
                if suffix.is_empty() {
                    assert_eq!(items[0].range.start, Position::new(0, 0));
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 18))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_entry_quote_boundaries_do_not_expose_quoted_line_fields() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        for quoted in [
            "'name'-line 700'",
            "'name'.line 700'",
            "'name':line 700'",
            "'name''line 700'",
            "'name';line 700'",
            "'name']line 700'",
        ] {
            let log =
                format!("WARN - Unexpected token {quoted} for entry 'entry' in file 'refs.bib'\n");
            let diagnostics = parse_bibliography(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(
                items[0].range,
                Range::new(Position::new(1, 6), Position::new(1, 11)),
                "{log}: {items:?}"
            );
            assert!(!unresolved_location(&items[0]));
        }
    }

    #[test]
    fn biber_databases_with_apostrophes_keep_entry_warning_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let bib = root.join("author's refs.bib");
        std::fs::write(&bib, "% database\n@book{example,title={Example}}\n").unwrap();
        let diagnostics = parse_bibliography(&root.join("main.tex"), "INFO - Found BibTeX data source 'author's refs.bib'\nWARN - Missing field for entry 'example'\nERROR - Error in 'author's refs.bib' line 2\n");
        let diagnostics = &diagnostics[&Url::from_file_path(bib).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(diagnostic.range.start.line, 1);
        assert_eq!(diagnostic.range.start.character, 6);
    }

    #[test]
    fn missing_include_messages_locate_paths_without_reporting_normal_missing_auxiliary_files() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("child.tex");
        std::fs::write(&main, "\\input{child}\n").unwrap();
        let source = "😀 \\include{missing chapter}\n% \\include{missing chapter}\n\\verb|\\include{missing chapter}|\n\\include{other}\n\\include{ missing chapter }\n";
        std::fs::write(&child, source).unwrap();
        let log = "(./main.tex\n(./child.tex\nNo file main.aux.\nNo file main.bbl.\nNo file optional.tex.\nNo file \"missing chapter.tex\".\nPackage local Warning: Unfinished message (\nNo file other.tex.\nNo file missing chapter.tex.\n)\nNo file optional.tex.\n)";
        let result = parse(&main, log);
        assert!(!result.contains_key(&Url::from_file_path(&main).unwrap()));
        let diagnostics = &result[&Url::from_file_path(&child).unwrap()];
        let missing: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("missing-include".into()))
            })
            .collect();
        assert_eq!(missing.len(), 3, "{diagnostics:?}");
        assert_eq!(missing[0].range.start.character, 12);
        let text = Text::new(source.into());
        assert_eq!(
            missing
                .iter()
                .map(
                    |diagnostic| &source[text.offset(diagnostic.range.start).unwrap()
                        ..text.offset(diagnostic.range.end).unwrap()]
                )
                .collect::<Vec<_>>(),
            vec!["missing chapter", "other", "missing chapter"]
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Package local Warning: Unfinished message ("));
        assert!(missing
            .iter()
            .all(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)));
    }

    #[test]
    fn wrapped_missing_includes_preserve_file_context_and_adjacent_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let child = root.join("child.tex");
        std::fs::write(&main, "\\input{child}\n\\include{parent}\n").unwrap();
        let chapter = format!("chapters/{} (中文 file)", "long-path-".repeat(12));
        let source = format!("😀 \\include{{{chapter}}}\n");
        std::fs::write(&child, &source).unwrap();
        for width in [16, 79, 121] {
            for quoted in [false, true] {
                let missing = if quoted {
                    format!("No file \"{chapter}.tex\".")
                } else {
                    format!("No file {chapter}.tex.")
                };
                let wrapped = missing
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(width)
                    .map(|chars| chars.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                let log = format!("(./main.tex\n(./child.tex\nNo file optional(broken).tex.\n{wrapped}\nNo file chapters/long-path-\nPackage local Warning: Adjacent warning on input line 1.\n)\nNo file parent.tex.\n)");
                let result = parse(&main, &log);
                let child_diagnostics = &result[&Url::from_file_path(&child).unwrap()];
                assert_eq!(child_diagnostics.len(), 2, "{result:?}");
                let missing = child_diagnostics
                    .iter()
                    .find(|diagnostic| missing_include_key(&diagnostic.message).is_some())
                    .unwrap();
                assert_eq!(
                    missing.message,
                    format!("Included file `{chapter}.tex` not found")
                );
                assert_eq!(
                    missing.range,
                    Range::new(
                        Position::new(0, 12),
                        Position::new(0, 12 + chapter.encode_utf16().count() as u32)
                    )
                );
                let parent = &result[&Url::from_file_path(&main).unwrap()];
                assert_eq!(parent.len(), 1);
                assert_eq!(parent[0].message, "Included file `parent.tex` not found");
                assert_eq!(parent[0].range.start.line, 1);
            }
        }
    }

    #[test]
    fn box_diagnostics_span_reported_source_lines_and_keep_valid_utf16_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\r\n  😀 second\r\n 中文 third\r\nLast\r\n").unwrap();
        let log = "(./main.tex\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--3\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--200\nOverfull \\vbox (2pt too high) detected at line 4\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--1\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 4);
        let find = |prefix: &str| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message.starts_with(prefix))
                .unwrap()
        };
        let overfull = find("Overfull \\hbox");
        assert_eq!(
            overfull.range,
            Range::new(Position::new(1, 2), Position::new(2, 9))
        );
        assert_eq!(
            overfull.code,
            Some(NumberOrString::String("overfull-box".into()))
        );
        let underfull = find("Underfull \\hbox (badness 10000) in paragraph at lines 3--200");
        assert_eq!(
            underfull.range,
            Range::new(Position::new(2, 1), Position::new(4, 0))
        );
        assert_eq!(
            underfull.code,
            Some(NumberOrString::String("underfull-box".into()))
        );
        assert_eq!(
            find("Overfull \\vbox").range,
            Range::new(Position::new(3, 0), Position::new(3, 4))
        );
        assert_eq!(
            find("Underfull \\hbox (badness 10000) in paragraph at lines 3--1").range,
            Range::new(Position::new(2, 1), Position::new(2, 9))
        );
    }

    #[test]
    fn single_quoted_citation_warnings_highlight_keys_and_preserve_apostrophes() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\autocite{first, O'Connor}\n\\ref{missing}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nPackage biblatex Warning: Citation 'O'Connor' on page 1 undefined on input line 1.\nPackage biblatex Warning: Citation 'first' undefined\n(biblatex) on input line 1.\nLaTeX Warning: Reference 'missing' on page 1 undefined on input line 2.\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 3);
        let text = Text::new(source.into());
        let values: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| {
                &source[text.offset(diagnostic.range.start).unwrap()
                    ..text.offset(diagnostic.range.end).unwrap()]
            })
            .collect();
        assert_eq!(values, vec!["first", "O'Connor", "missing"]);
        assert_eq!(diagnostics[0].range.start.character, 13);
    }

    #[test]
    fn bibliography_line_errors_keep_the_database_and_ambiguous_entry_warnings_keep_the_root() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let bib = directory.path().join("with spaces.bib");
        std::fs::write(&main, "\\cite{example}\n").unwrap();
        std::fs::write(
            &bib,
            "\n@book{example,\n title={Example}\n}\n@book{example, title={Duplicate}}\n",
        )
        .unwrap();
        let log = "Database file #1: with spaces.bib\nRepeated entry---line 5 of file with spaces.bib\nI was expecting a `,' or a `}'---line 3 of file with spaces.bib\nWarning--empty author in example\n";
        let diagnostics = parse_bibliography(&main, log);
        let entries = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].range.start.line, 2);
        assert_eq!(entries[1].range.start.line, 4);
        let warning = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(warning.range.start, Position::new(0, 0));
        assert_eq!(warning.severity, Some(DiagnosticSeverity::WARNING));
        let log = "[0] INFO - Found BibTeX data source 'with spaces.bib'\n[1] ERROR - syntax error in file 'with spaces.bib', line 3\n[2] WARN - Missing required field for entry 'example'\n";
        let diagnostics = parse_bibliography(&main, log);
        let entries = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].range.start.line, 2);
        assert!(entries[0].message.starts_with("Biber:"));
        let warning = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(warning.range.start.line, 0);
        assert_eq!(warning.severity, Some(DiagnosticSeverity::WARNING));
        let diagnostics = parse_bibliography(&main,"I couldn't open database file missing.bib\n---line 4 of file main.aux\nI found no database files---while reading file main.aux\n");
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("couldn't open database file missing.bib")));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 0));
    }

    #[test]
    fn file_line_errors_and_multiline_warnings_use_the_right_file() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("章节.tex");
        std::fs::write(&main, "\\input{章节}\n\\ref{missing}\n").unwrap();
        std::fs::write(&chapter, "hello\n😀 \\bad\n").unwrap();
        let log = "(./main.tex\n(./章节.tex\n./章节.tex:2: Undefined control sequence.\nl.2 \\bad\n)\nLaTeX Warning: Reference `missing' on page 1 undefined on input\n line 2.\n\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--2\n)";
        let diagnostics = parse(&main, log);
        let chapter = &diagnostics[&Url::from_file_path(chapter).unwrap()];
        assert_eq!(chapter.len(), 1);
        assert_eq!(chapter[0].range.start.line, 1);
        assert_eq!(chapter[0].range.end.character, 7);
        let main = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(main.len(), 2);
        assert!(main
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
    }

    #[test]
    fn font_class_engine_and_glyph_warnings_preserve_adjacent_errors_and_deduplicate_shipout_messages(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\nSecond\nThird\n").unwrap();
        let log = "(./main.tex\nClass memoir Warning: Option ignored\nLaTeX Font Warning: Font shape unavailable\n(Font) using a substitute on input line 3.\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\npdfTeX warning (dest): destination missing!\n! First error.\n! Second error.\nl.2 bad\n)\n";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 6, "{diagnostics:?}");
        let font = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.starts_with("LaTeX Font Warning:"))
            .unwrap();
        assert_eq!(font.range.start.line, 2);
        assert!(font.message.contains("(Font) using a substitute"));
        let glyph = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("Missing character:"))
            .collect::<Vec<_>>();
        assert_eq!(glyph.len(), 1);
        assert!(glyph[0].message.ends_with("Latin Modern Roman!"));
        assert_eq!(glyph[0].severity, Some(DiagnosticSeverity::WARNING));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Class memoir Warning: Option ignored"));
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                .count(),
            2
        );
    }

    #[test]
    fn traditional_errors_and_quoted_paths_are_supported() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("with spaces.tex");
        std::fs::write(&main, "main\n").unwrap();
        std::fs::write(&chapter, "hello\n\\bad\n").unwrap();
        let diagnostics = parse(
            &main,
            "(./main.tex\n(\"./with spaces.tex\"\n! Undefined control sequence.\nl.2 \\bad\n)\n)",
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(chapter).unwrap()][0]
                .range
                .start
                .line,
            1
        );
    }

    #[test]
    fn source_paths_and_diagnostic_parentheses_preserve_nested_file_context() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let definition = directory.path().join("配置 (旧).def");
        let options = directory.path().join("layout options.clo");
        for path in [&main, &definition, &options] {
            std::fs::write(path, "One\n😀 Two\nThree\nFour\nFive\n").unwrap();
        }
        let log = "(./main.tex\n(./配置 (旧).def\nPackage local Warning: Closing ) marker on input line 2.\nPackage local Warning: Still local on input line 3.\n! Error with a ( marker.\nl.4 \\bad)\nPackage local Warning: After error on input line 5.\n)\n(./layout options.clo\nLaTeX Warning: Options warning on input line 2.\n)\nLaTeX Warning: Back in main on input line 3.\n)";
        let diagnostics = parse(&main, log);
        let local = &diagnostics[&Url::from_file_path(&definition).unwrap()];
        assert_eq!(local.len(), 4, "{diagnostics:?}");
        assert_eq!(local[0].range.end.character, 6);
        assert!(local
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 3
                && diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
        assert_eq!(
            diagnostics[&Url::from_file_path(&options).unwrap()][0]
                .range
                .start
                .line,
            1
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(&main).unwrap()][0]
                .range
                .start
                .line,
            2
        );
    }

    #[test]
    fn undefined_references_highlight_keys_in_lists_multiline_calls_and_utf16_text() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\cref{found, missing} \\cite{lost}\n% \\ref{missing}\n\\verb|\\ref{missing}|\n\\ref\n{missing}\n\\ref{O'Connor}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nPackage natbib Warning: Citation `lost' on page 1 undefined on input line 1.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 2.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 3.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 5.\nLaTeX Warning: Reference `O'Connor' undefined on input line 6.\n)";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 7, "{diagnostics:?}");
        let text = Text::new(source.into());
        let value = |range: Range| {
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()]
        };
        let first: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.start.line == 0)
            .collect();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].range.start.character, 8);
        assert_eq!(
            first
                .iter()
                .map(|diagnostic| value(diagnostic.range))
                .collect::<Vec<_>>(),
            vec!["missing", "missing", "lost"]
        );
        assert_eq!(value(diagnostics[3].range), "% \\ref{missing}");
        assert_eq!(value(diagnostics[4].range), "\\verb|\\ref{missing}|");
        assert_eq!(value(diagnostics[5].range), "missing");
        assert_eq!(value(diagnostics[6].range), "O'Connor");
    }

    #[test]
    fn single_reference_ranges_do_not_normalize_whitespace_but_cref_lists_keep_key_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\ref{ missing } \\cref{found, missing }\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nLaTeX Warning: Reference ` missing ' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 3, "{errors:?}");
        let text = Text::new(source.into());
        let precise: Vec<_> = errors
            .iter()
            .filter(|error| !error.message.contains("` missing '"))
            .collect();
        assert_eq!(precise.len(), 2);
        for (error, start) in precise.iter().zip([
            source.find("missing").unwrap(),
            source.rfind("missing").unwrap(),
        ]) {
            assert_eq!(error.range, text.range(start, start + 7));
        }
        let fallback = errors
            .iter()
            .find(|error| error.message.contains("` missing '"))
            .unwrap();
        assert_eq!(fallback.range, text.range(0, source.len() - 1));
    }

    #[test]
    fn silent_refcount_queries_do_not_inherit_warnings_from_another_call_on_the_line() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = concat!(
            "\\refused{missing} \\getrefnumber{missing} \\getpagerefnumber{missing} ",
            "\\getrefbykeydefault{missing}{page}{0} ",
            "\\IfRefUndefinedExpandable{missing}{A}{B} \\IfRefUndefinedBabel{missing}{A}{B}\n",
        );
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1, "{errors:?}");
        let start = source.find("missing").unwrap();
        assert_eq!(
            errors[0].range,
            Text::new(source.into()).range(start, start + 7)
        );
    }

    #[test]
    fn wrapped_file_announcements_rejoin_existing_names_without_swallowing_warnings() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("settings {}(old).def", "long-name-".repeat(12));
        let definition = directory.path().join(&name);
        std::fs::write(&main, "Main\n").unwrap();
        std::fs::write(&definition, "First\nSecond\n").unwrap();
        for quoted in [false, true] {
            let opening = if quoted {
                format!("(\"./{name}\"")
            } else {
                format!("(./{name}")
            };
            let wrapped = format!("{}\n{}", &opening[..79], &opening[79..]);
            let log = format!("(./main.tex\n{wrapped}\nLaTeX Warning: Local warning on input line 2.\n)\nLaTeX Warning: Root warning on input line 1.\n)\n");
            let diagnostics = parse(&main, &log);
            assert_eq!(
                diagnostics[&Url::from_file_path(&definition).unwrap()][0]
                    .range
                    .start
                    .line,
                1
            );
            assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
        }
        let log = format!(
            "(./main.tex\n({}\nLaTeX Warning: Do not swallow this on input line 1.\n",
            "not-a-file-".repeat(9)
        );
        let diagnostics = parse(&main, &log);
        assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
    }
}

#[cfg(test)]
mod bibliography_style_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n",
        )
    }

    #[test]
    fn style_locations_use_only_confirmed_sources_and_preserve_ambiguous_failures() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let unread = root.join("unread.tex");
        std::fs::write(&main, "\\input{settings}\n").unwrap();
        let source = "% emoji😀\r\n\\bibliographystyle{% before\r\n local,styel% after\r\n}\r\n";
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find(" local,styel").unwrap() + 1;
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "local,styel".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"local,styel.bst"}))
        );

        let mut sources = sources;
        sources.insert(unread_uri);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&settings_uri));
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&unread, r"\bibliographystyle{\chosen}").unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }

    #[test]
    fn unavailable_oversized_or_truncated_style_sources_do_not_create_false_unique_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliographystyle{local,styel}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [main.clone(), settings, other.clone()]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&main, r"\bibliographystyle{local,styel}").unwrap();
        let log = format!(
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n{}\n",
            crate::process::output::TRUNCATED
        );
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }
}

#[cfg(test)]
mod bibliography_database_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n",
        )
    }

    #[test]
    fn databases_use_confirmed_sources_including_local_packages_and_reject_ambiguity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, r"\usepackage{settings}").unwrap();
        let source = "\\bibliography{first,% before\r\n re ffs.bib% after\r\n,second}";
        let settings = root.join("settings.sty");
        let unread = root.join("unread.tex");
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let mut sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find("re ffs").unwrap();
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "re ffs.bib".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
        );
        sources.insert(unread_uri);
        for source in [
            source,
            r"\nobibliography{\chosen}",
            r"\renewcommand{\bibliography}[1]{}",
        ] {
            std::fs::write(&unread, source).unwrap();
            let mut diagnostics = missing(&main);
            locate_missing_bibliography_databases(&sources, &mut diagnostics);
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            );
        }
    }

    #[test]
    fn incomplete_database_sources_keep_unresolved_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliography{reffs}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [&main, &settings, &other]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let assert_unresolved = |diagnostics: &BTreeMap<Url, Vec<Diagnostic>>| {
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            )
        };
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&main, r"\bibliography{reffs}").unwrap();
        let log = format!(
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n{}\n",
            crate::process::output::TRUNCATED
        );
        assert_unresolved(&parse_bibliography(&main, &log));
    }

    #[test]
    fn style_and_database_uncertainty_is_independent() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let log="I couldn't open database file reffs.bib\n---line 3 of file main.aux\nI couldn't open style file styel.bst\n---line 2 of file main.aux\n";
        for (source, precise, unresolved) in [
            (
                r"\bibliographystyle{styel}\bibliography{\chosen}",
                "missing-bibliography-style",
                "missing-bibliography-database",
            ),
            (
                r"\bibliographystyle{\chosen}\bibliography{reffs}",
                "missing-bibliography-database",
                "missing-bibliography-style",
            ),
        ] {
            std::fs::write(&main, source).unwrap();
            let diagnostics = parse_bibliography(&main, log);
            let find = |code: &str| {
                diagnostics
                    .values()
                    .flatten()
                    .find(|item| item.code == Some(NumberOrString::String(code.into())))
                    .unwrap()
            };
            assert!(!unresolved_location(find(precise)));
            assert!(unresolved_location(find(unresolved)));
        }
    }
}

#[cfg(test)]
mod latexmk_database_warnings {
    use super::*;

    fn section(found: &[&str], missing: &[&str]) -> String {
        let mut output = "Latexmk: Found bibliography file(s):\n".to_string();
        for name in found {
            output.push_str(&format!("  {name}\n"));
        }
        if !missing.is_empty() {
            output.push_str("Bib file(s) not found in search path:\n");
            for name in missing {
                output.push_str(&format!("  {name}\n"));
            }
        }
        output.push_str("Latexmk: All targets are up-to-date\n");
        output
    }
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let log = directory.path().join("out (local)");
        std::fs::create_dir(&log).unwrap();
        let log = log.join("main's.log");
        std::fs::write(
            &root,
            "% 😀 before\r\n\\bibliography{first, re ffs.bib,second}\r\n",
        )
        .unwrap();
        std::fs::write(&log, "Current TeX log.\n").unwrap();
        (directory, root, log)
    }
    fn inspect(log: &str, tail: &str) -> String {
        format!("Transcript written on {log}.\nLatexmk: Getting log file '{log}'\nLatexmk: Examining '{log}'\n{tail}")
    }

    #[test]
    fn final_driver_lists_create_bounded_warnings_and_support_exact_source_lookup() {
        let (_directory, root, log) = setup();
        let tail = section(
            &["first.bib", "reffs.bib", "second.bib"],
            &["reffs.bib", "reffs.bib"],
        );
        let with_blank_line = tail.replace("\nLatexmk: All targets", "\n\nLatexmk: All targets");
        assert!(!parse_latexmk_missing_databases(
            &root,
            &log,
            &inspect("out (local)/main's.log", &with_blank_line)
        )
        .is_empty());
        for printed in [
            "out (local)/main's.log".to_string(),
            log.display().to_string(),
        ] {
            let mut diagnostics =
                parse_latexmk_missing_databases(&root, &log, &inspect(&printed, &tail));
            let uri = Url::from_file_path(&root).unwrap();
            assert_eq!(diagnostics[&uri].len(), 1);
            let item = &diagnostics[&uri][0];
            assert_eq!(
                item.message,
                "Latexmk: Bibliography database not found: reffs.bib"
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert!(unresolved_location(item));
            locate_missing_bibliography_databases(&BTreeSet::from([uri.clone()]), &mut diagnostics);
            let item = &diagnostics[&uri][0];
            let source = std::fs::read_to_string(&root).unwrap();
            let start = source.find("re ffs.bib").unwrap();
            assert_eq!(
                item.range,
                Text::new(source).range(start, start + "re ffs.bib".len())
            );
            assert_eq!(
                item.data,
                Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
            );
        }
    }

    #[test]
    fn earlier_passes_user_messages_other_jobs_and_child_tool_output_do_not_supply_warnings() {
        let (_directory, root, log) = setup();
        let missing = section(&["reffs.bib"], &["reffs.bib"]);
        let recovered = section(&["./refs.bib"], &[]);
        let marker = inspect("out (local)/main's.log", "");
        for output in [
            missing.clone(),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\nUnexpected tool output\n"),
            format!("{missing}{marker}Latexmk: All targets are up-to-date\n"),
            format!("{marker}{missing}{marker}{recovered}"),
            format!("{marker}{missing}{recovered}"),
            inspect("other.log",&missing),
            format!("{marker}Latexmk: Getting log file 'other.log'\n{missing}"),
            format!("{marker}Latexmk: applying rule 'bibtex main'...\n{missing}"),
            format!("{marker}Running 'unrelated compiler'\n{missing}"),
            format!("{marker}Latexmk: Bibliography file(s) from .bcf file:\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Bib file(s) not found in search path:\n  reffs.bib\n"),
        ] {
            assert!(parse_latexmk_missing_databases(&root,&log,&output).is_empty(),"{output}");
        }
    }

    #[test]
    fn incomplete_invalid_or_unconfirmed_driver_names_do_not_create_partial_warnings() {
        let (_directory, root, log) = setup();
        for (found, missing) in [
            (vec!["reffs.bib"], vec!["other.bib"]),
            (
                vec!["reffs.bib", "wrong.tex"],
                vec!["reffs.bib", "wrong.tex"],
            ),
            (
                vec!["reffs.bib", "spaced name.bib"],
                vec!["reffs.bib", "spaced name.bib"],
            ),
            (
                vec!["reffs.bib", r"\chosen.bib"],
                vec!["reffs.bib", r"\chosen.bib"],
            ),
            (vec![".bib"], vec![".bib"]),
            (vec![], vec!["reffs.bib"]),
        ] {
            let output = inspect("out (local)/main's.log", &section(&found, &missing));
            assert!(
                parse_latexmk_missing_databases(&root, &log, &output).is_empty(),
                "{output}"
            );
        }
        let output = inspect(
            "out (local)/main's.log",
            &section(&["reffs.bib"], &["reffs.bib"]),
        );
        let truncated = format!("{output}{}\n", crate::process::output::TRUNCATED);
        assert!(parse_latexmk_missing_databases(&root, &log, &truncated).is_empty());
        std::fs::remove_file(&log).unwrap();
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }

    #[test]
    fn driver_database_lists_reject_excessive_counts_and_names() {
        let (_directory, root, log) = setup();
        for (count, limit) in [(65, 64), (4097, 0)] {
            let names: Vec<_> = (0..count).map(|n| format!("refs{n}.bib")).collect();
            let found: Vec<_> = names.iter().map(String::as_str).collect();
            let missing = if limit == 0 { &found[..1] } else { &found[..] };
            let output = inspect("out (local)/main's.log", &section(&found, missing));
            assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
        }
        let name = format!("{}.bib", "r".repeat(8193));
        let output = inspect("out (local)/main's.log", &section(&[&name], &[&name]));
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }
}

#[cfg(test)]
mod project_color_tests {
    use super::*;

    const HELP: &str = "See the xcolor package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";
    const COUNTER_HELP: &str = "See the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...\n\n";

    fn fixture(definitions: &str, call: &str) -> (tempfile::TempDir, PathBuf, String) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main.tex");
        let source = format!("\\usepackage{{palette}}\n{call}\n");
        std::fs::write(&root, &source).unwrap();
        std::fs::write(temp.path().join("palette.sty"), definitions).unwrap();
        (temp, root, source)
    }

    fn log(call: &str, model: bool, before: &str, after: &str) -> String {
        let message = if model {
            "Undefined color model `rbg'."
        } else {
            "Undefined color `accnet'."
        };
        format!("(./main.tex\n{before}\n./main.tex:2: Package xcolor Error: {message}\n{HELP}l.2 {call}\n{after}\n)\n")
    }

    fn error(root: &Path, log: &str, project: &ProjectSources) -> Diagnostic {
        parse_with_project_sources(root, log, project).0[&Url::from_file_path(root).unwrap()]
            .iter()
            .find(|d| d.message.starts_with("Package xcolor Error:"))
            .unwrap()
            .clone()
    }

    #[test]
    fn loaded_saved_dependencies_supply_arities_and_alias_target_grammars() {
        for (definitions, prefix) in [
            (r"\newcommand{\bridge}{}", r"\bridge"),
            (r"\newcommand{\done}{}\def\bridge{\done}", r"\bridge"),
            (
                r"\NewDocumentCommand{\done}{}{}\NewDocumentCommand{\bridge}{}{\done}",
                r"\bridge",
            ),
            (
                r"\newcommand{\done}[1]{}\def\bridge{\done}",
                r"\bridge{Unused}",
            ),
            (
                r"\newcommand{\done}[1][default]{}\def\bridge{\done}",
                r"\bridge[note]",
            ),
            (
                r"\NewDocumentCommand{\done}{s t+ O{default} m}{}\def\bridge{\done}",
                r"\bridge*+[note]{Unused}",
            ),
            (
                r"\NewDocumentCommand{\done}{d() m}{}\def\bridge{\done}",
                r"\bridge(note){Unused}",
            ),
        ] {
            for model in [false, true] {
                let call = format!(
                    "{prefix}{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (_temp, root, source) = fixture(definitions, &call);
                let project = ProjectSources::capture(&root);
                let log = log(&call, model, "(./palette.sty)", "");
                let diagnostic = error(&root, &log, &project);
                let name = if model { "rbg" } else { "accnet" };
                let start = source.find(name).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source).range(start, start + name.len()),
                    "{definitions}/{call}"
                );
                assert_eq!(
                    diagnostic.code,
                    Some(NumberOrString::String(
                        if model {
                            "undefined-color-model"
                        } else {
                            "undefined-color"
                        }
                        .into()
                    ))
                );
            }
        }
    }

    #[test]
    fn repeated_color_and_model_errors_follow_changes_in_the_loaded_macro_scope() {
        let color = r"\bridge\textcolor{accnet}{MarkerUnique}";
        let model = r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
        let (temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let calls = [color, model, color, model, r"\input{later}", color, model];
        let source = format!("\\usepackage{{palette}}\n{}\n", calls.join("\n"));
        std::fs::write(&root, &source).unwrap();
        std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[3]{}").unwrap();
        let project = ProjectSources::capture(&root);
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for (index, call) in calls.iter().enumerate() {
            if index == 4 {
                log.push_str("(./later.tex)\n");
                continue;
            }
            let line = index + 2;
            let name = if *call == color {
                "color `accnet'"
            } else {
                "color model `rbg'"
            };
            log.push_str(&format!("./main.tex:{line}: Package xcolor Error: Undefined {name}.\n{HELP}l.{line} {call}\n"));
        }
        log.push_str(")\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        let diagnostics = &result[&Url::from_file_path(&root).unwrap()];
        assert_eq!(diagnostics.len(), 6);
        let text = Text::new(source);
        for diagnostic in diagnostics {
            if diagnostic.range.start.line < 5 {
                let name = if diagnostic.range.start.line % 2 == 1 {
                    "accnet"
                } else {
                    "rbg"
                };
                let line_start = text
                    .offset(Position::new(diagnostic.range.start.line, 0))
                    .unwrap();
                let start = line_start + text.source[line_start..].find(name).unwrap();
                assert_eq!(diagnostic.range, text.range(start, start + name.len()));
                assert!(diagnostic.code.is_some());
                let sources = diagnostic.data.as_ref().unwrap()["macroSources"]
                    .as_array()
                    .unwrap();
                assert_eq!(sources.len(), 2);
            } else {
                assert!(
                    diagnostic.code.is_none() && diagnostic.data.is_none(),
                    "{diagnostic:?}"
                );
            }
        }
    }

    #[test]
    fn project_counter_errors_require_a_complete_error_time_macro_scope() {
        let call = r"\bridge\setcounter{equatoin}{7}";
        for case in [
            "direct",
            "callback",
            "not-loaded",
            "later",
            "displayed",
            "dynamic",
        ] {
            let declarations = if case == "callback" {
                r"\newcommand{\done}[3]{}\def\bridge{\done}"
            } else {
                r"\newcommand{\done}{}\def\bridge{\done}"
            };
            let (temp, root, _) = fixture(declarations, call);
            let prefix = match case {
                "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                "dynamic" => r"\usepackage{palette}\input{\filename}",
                _ => r"\usepackage{palette}",
            };
            let source = format!("{prefix}\n{call}\n\\input{{later}}\n");
            std::fs::write(&root, &source).unwrap();
            std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[3]{}").unwrap();
            // A display-only file must not also have an explicit source role.
            if case == "displayed" {
                std::fs::write(&root, format!("{prefix}\n{call}\n")).unwrap();
            }
            let project = ProjectSources::capture(&root);
            let before = if case == "not-loaded" {
                ""
            } else {
                "(./palette.sty)"
            };
            let before = format!(
                "{before}{}",
                if case == "displayed" {
                    "\n(./later.tex)"
                } else {
                    ""
                }
            );
            let log = format!("(./main.tex\n{before}\n./main.tex:2: LaTeX Error: No counter 'equatoin' defined.\n{COUNTER_HELP}l.2 {call}\n(./later.tex)\n)\n");
            let result = parse_with_project_sources(&root, &log, &project).0;
            let error = &result[&Url::from_file_path(&root).unwrap()][0];
            let precise = matches!(case, "direct" | "later" | "displayed");
            assert_eq!(error.code.is_some(), precise, "{case}: {error:?}");
            if precise {
                assert_eq!(error.data.as_ref().unwrap()["counter"], "equatoin");
                let sources: Vec<Url> =
                    serde_json::from_value(error.data.as_ref().unwrap()["macroSources"].clone())
                        .unwrap();
                assert_eq!(sources.len(), 2);
                assert!(
                    !sources.contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap())
                );
            } else {
                assert!(error.data.is_none(), "{case}: {error:?}");
            }
        }
    }

    #[test]
    fn repeated_color_model_and_counter_errors_keep_separate_call_analyses() {
        let calls = [
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..9 {
            let (call, message, _) = calls[index % 3];
            let line = index + 2;
            let help = if index % 3 == 2 { COUNTER_HELP } else { HELP };
            source.push_str(&format!("{call}\n"));
            log.push_str(&format!(
                "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
            ));
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors = &result[&Url::from_file_path(root).unwrap()];
        assert_eq!(errors.len(), 9);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % 3];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(error.range, text.range(start, start + name.len()));
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn project_environment_errors_require_error_time_macro_sources() {
        for command in ["begin", "end"] {
            let call = format!(r"\bridge\{command}{{algin}}");
            let message = if command == "begin" {
                "LaTeX Error: Environment algin undefined."
            } else {
                r"LaTeX Error: \begin{equation} ended by \end{algin}."
            };
            for case in [
                "direct",
                "callback",
                "not-loaded",
                "later",
                "displayed",
                "dynamic",
            ] {
                let declarations = if case == "callback" {
                    r"\newcommand{\done}[2]{}\def\bridge{\done}"
                } else {
                    r"\newcommand{\done}{}\def\bridge{\done}"
                };
                let (temp, root, _) = fixture(declarations, &call);
                let prefix = match case {
                    "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                    "dynamic" => r"\usepackage{palette}\input{\filename}",
                    _ => r"\usepackage{palette}",
                };
                let source = if case == "displayed" {
                    format!("{prefix}\n{call}\n")
                } else {
                    format!("{prefix}\n{call}\n\\input{{later}}\n")
                };
                std::fs::write(&root, source).unwrap();
                std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[2]{}")
                    .unwrap();
                let project = ProjectSources::capture(&root);
                let before = if case == "not-loaded" {
                    ""
                } else {
                    "(./palette.sty)"
                };
                let display = if case == "displayed" {
                    "(./later.tex)"
                } else {
                    ""
                };
                let log = format!("(./main.tex\n{before}\n{display}\n./main.tex:2: {message}\n{COUNTER_HELP}l.2 {call}\n(./later.tex)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let error = &result[&Url::from_file_path(&root).unwrap()][0];
                let precise = matches!(case, "direct" | "later" | "displayed");
                assert_eq!(error.code.is_some(), precise, "{command}/{case}: {error:?}");
                if precise {
                    let sources: Vec<Url> = serde_json::from_value(
                        error.data.as_ref().unwrap()["macroSources"].clone(),
                    )
                    .unwrap();
                    assert_eq!(sources.len(), 2);
                    assert!(!sources
                        .contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap()));
                } else {
                    assert!(error.data.is_none(), "{command}/{case}: {error:?}");
                }
            }
        }
    }

    #[test]
    fn interleaved_invocation_errors_keep_five_independent_call_caches() {
        let calls = [
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
            (
                r"\bridge\begin{panle}",
                "LaTeX Error: Environment panle undefined.",
                "panle",
            ),
            (
                r"\bridge\end{algin}",
                r"LaTeX Error: \begin{document} ended by \end{algin}.",
                "algin",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..15 {
            let (call, message, _) = calls[index % 5];
            let line = index + 2;
            let help = if index % 5 < 2 { HELP } else { COUNTER_HELP };
            source.push_str(&format!("{call}\n"));
            log.push_str(&format!(
                "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
            ));
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors = &result[&Url::from_file_path(root).unwrap()];
        assert_eq!(errors.len(), 15);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % 5];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(error.range, text.range(start, start + name.len()));
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn project_file_errors_require_macros_loaded_before_the_failure() {
        for command in ["input", "usepackage"] {
            let call = format!(r"\bridge\{command}{{palete}}\relax");
            let echo = if command == "input" {
                call.strip_suffix(r"\relax").unwrap()
            } else {
                &call
            };
            let extension = if command == "input" { "tex" } else { "sty" };
            for case in [
                "direct",
                "callback",
                "not-loaded",
                "later",
                "displayed",
                "dynamic",
                "dynamic-definition",
            ] {
                let declarations = if case == "callback" {
                    r"\newcommand{\done}[2]{}\def\bridge{\done}"
                } else {
                    r"\newcommand{\done}{}\def\bridge{\done}"
                };
                let (temp, root, _) = fixture(declarations, &call);
                let prefix = match case {
                    "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                    "dynamic" => r"\usepackage{palette}\input{\filename}",
                    "dynamic-definition" => {
                        r"\usepackage{palette}\newcommand{\unused}[1]{\input{#1}}"
                    }
                    _ => r"\usepackage{palette}",
                };
                let source = if case == "displayed" {
                    format!("{prefix}\n{call}\n")
                } else {
                    format!("{prefix}\n{call}\n\\input{{later}}\n")
                };
                std::fs::write(&root, source).unwrap();
                std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[2]{}")
                    .unwrap();
                let project = ProjectSources::capture(&root);
                let before = if case == "not-loaded" {
                    ""
                } else {
                    "(./palette.sty)"
                };
                let display = if case == "displayed" {
                    "(./later.tex)"
                } else {
                    ""
                };
                let log = format!("(./main.tex\n{before}\n{display}\n! LaTeX Error: File `palete.{extension}' not found.\n./main.tex:2: Emergency stop.\n<read *>\nl.2 {echo}\n(./later.tex)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let error = result[&Url::from_file_path(&root).unwrap()]
                    .iter()
                    .find(|e| e.message.contains("not found"))
                    .unwrap();
                let precise = matches!(case, "direct" | "later" | "displayed");
                assert_eq!(error.code.is_some(), precise, "{command}/{case}: {error:?}");
                if precise {
                    let sources: Vec<Url> = serde_json::from_value(
                        error.data.as_ref().unwrap()["macroSources"].clone(),
                    )
                    .unwrap();
                    assert_eq!(sources.len(), 2);
                    assert!(!sources
                        .contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap()));
                } else {
                    assert!(error.data.is_none());
                }
            }
        }
    }

    #[test]
    fn file_input_and_loader_errors_share_only_the_file_call_cache() {
        let calls = [
            (
                r"\bridge\input{palete}",
                "LaTeX Error: File `palete.tex' not found.",
                "palete",
            ),
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\usepackage{palete}\relax",
                "LaTeX Error: File `palete.sty' not found.",
                "palete",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
            (
                r"\bridge\begin{panle}",
                "LaTeX Error: Environment panle undefined.",
                "panle",
            ),
            (
                r"\bridge\end{algin}",
                r"LaTeX Error: \begin{document} ended by \end{algin}.",
                "algin",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\deflength{\measuer}{1pt}",
                "Package etoolbox Error: Length '\\measuer' undefined.",
                r"\measuer",
            ),
            (
                r"\bridge\settoggle{darft}{true}",
                "Package etoolbox Error: Toggle 'darft' undefined.",
                "darft",
            ),
            (
                r"\bridge\setbool{darft}{true}",
                "Package etoolbox Error: Boolean '\\ifdarft' undefined.",
                "darft",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..calls.len() * 2 {
            let (call, message, _) = calls[index % calls.len()];
            let line = index + 2;
            source.push_str(&format!("{call}\n"));
            if matches!(index % calls.len(), 0 | 2) {
                log.push_str(&format!(
                    "! {message}\n./main.tex:{line}: Emergency stop.\n<read *>\nl.{line} {call}\n"
                ));
            } else {
                let help = if matches!(index % calls.len(), 1 | 6) {
                    HELP
                } else {
                    COUNTER_HELP
                };
                log.push_str(&format!(
                    "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
                ));
            }
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors: Vec<_> = result[&Url::from_file_path(root).unwrap()]
            .iter()
            .filter(|e| e.message != "Emergency stop.")
            .collect();
        assert_eq!(errors.len(), calls.len() * 2);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % calls.len()];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(
                error.range,
                text.range(start, start + name.len()),
                "{index}: {error:?}"
            );
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn returning_to_a_loaded_file_keeps_each_documents_color_ranges() {
        let (temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        std::fs::write(
            &root,
            r"\usepackage{palette}\input{first}\input{second}\input{first}",
        )
        .unwrap();
        let color = r"\bridge\textcolor{accnet}{MarkerUnique}";
        let model = r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
        let mut documents = BTreeMap::new();
        for (name, prefix) in [("first", ""), ("second", r"\typeout{😀} ")] {
            let source = format!("{prefix}{color}\n{prefix}{model}\n");
            let path = temp.path().join(format!("{name}.tex"));
            std::fs::write(&path, &source).unwrap();
            documents.insert(name, Text::new(source));
        }
        let project = ProjectSources::capture(&root);
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for name in ["first", "second", "first"] {
            log.push_str(&format!("(./{name}.tex\n"));
            for (index, (call, message)) in
                [(color, "color `accnet'"), (model, "color model `rbg'")]
                    .into_iter()
                    .enumerate()
            {
                let prefix = if name == "first" {
                    ""
                } else {
                    r"\typeout{😀} "
                };
                let line = index + 1;
                log.push_str(&format!("./{name}.tex:{line}: Package xcolor Error: Undefined {message}.\n{HELP}l.{line} {prefix}{call}\n"));
            }
            log.push_str(")\n");
        }
        log.push_str(")\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        for (name, expected_count) in [("first", 4), ("second", 2)] {
            let uri = Url::from_file_path(temp.path().join(format!("{name}.tex"))).unwrap();
            let diagnostics = &result[&uri];
            assert_eq!(diagnostics.len(), expected_count);
            let text = &documents[name];
            for diagnostic in diagnostics {
                let name = if diagnostic.range.start.line == 0 {
                    "accnet"
                } else {
                    "rbg"
                };
                let start = text.source.find(name).unwrap();
                assert_eq!(diagnostic.range, text.range(start, start + name.len()));
                assert!(diagnostic.code.is_some());
            }
        }
    }

    #[test]
    fn reused_analysis_rechecks_sources_and_rejects_a_different_emission_text() {
        let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
        for paused_parent in [false, true] {
            let (temp, root, original) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", call);
            let child = temp.path().join("content.tex");
            std::fs::write(&child, call).unwrap();
            if paused_parent {
                std::fs::write(&root, r"\usepackage{palette}\input{content}").unwrap();
            }
            let project = ProjectSources::capture(&root);
            let mut analysis = ProjectAnalysis::new(&project);
            let root_uri = Url::from_file_path(&root).unwrap();
            let uri = if paused_parent {
                Url::from_file_path(&child).unwrap()
            } else {
                root_uri.clone()
            };
            let palette_uri = Url::from_file_path(temp.path().join("palette.sty")).unwrap();
            let loaded = [uri.clone(), palette_uri].into();
            let paused = if paused_parent {
                [root_uri].into()
            } else {
                BTreeSet::new()
            };
            let context = analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .unwrap()
                .unwrap();
            let source = if paused_parent { call } else { &original };
            let line = u32::from(!paused_parent);
            assert!(color_error_range(
                &Text::new(source.into()),
                line,
                call,
                "accnet",
                false,
                Some(&context.calls)
            )
            .is_some());
            assert!(color_error_range(
                &Text::new(format!("{source}\n% changed")),
                line,
                call,
                "accnet",
                false,
                Some(&context.calls)
            )
            .is_none());
            let saved_root = std::fs::read_to_string(&root).unwrap();
            std::fs::write(
                &root,
                format!("{saved_root}\n\\renewcommand{{\\done}}[3]{{}}"),
            )
            .unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .is_err());
            std::fs::write(&root, saved_root).unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .unwrap()
                .is_some());
            std::fs::remove_file(temp.path().join("palette.sty")).unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .is_err());
        }
    }

    #[test]
    fn callbacks_and_redefinitions_in_other_files_cannot_become_direct_color_sites() {
        for definitions in [
            r"\newcommand{\wrapped}[3]{}\def\bridge{\wrapped}",
            r"\newcommand{\bridge}{}\renewcommand{\bridge}[3]{}",
            r"\def\bridge{\missing}",
            r"\def\bridge{\bridge}",
            r"\newcommand{\bridge}{}\renewcommand{\textcolor}[2]{}",
        ] {
            for model in [false, true] {
                let call = format!(
                    r"\bridge{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (_temp, root, _) = fixture(definitions, &call);
                let project = ProjectSources::capture(&root);
                let diagnostic = error(&root, &log(&call, model, "(./palette.sty)", ""), &project);
                assert!(
                    diagnostic.code.is_none() && diagnostic.data.is_none(),
                    "{definitions}/{call}: {diagnostic:?}"
                );
            }
        }
    }

    #[test]
    fn source_graph_requires_prior_log_entry_and_ignores_unrelated_files() {
        for (before, after) in [("", ""), ("", "(./palette.sty)"), ("(./unrelated.sty)", "")] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, _) = fixture(r"\def\bridge{}", call);
            std::fs::write(temp.path().join("unrelated.sty"), r"\def\bridge{}").unwrap();
            let project = ProjectSources::capture(&root);
            let diagnostic = error(&root, &log(call, false, before, after), &project);
            assert!(
                diagnostic.code.is_none(),
                "{before}/{after}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn later_inputs_and_displayed_code_do_not_override_the_error_time_signature_scope() {
        for displayed in [false, true] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, source) = fixture(r"\def\bridge{}", call);
            let extra = temp.path().join("extra.tex");
            std::fs::write(&extra, r"\renewcommand{\bridge}[3]{}").unwrap();
            let source = if displayed {
                format!("\\usepackage{{palette}}\\lstinputlisting{{extra.tex}}\n{call}\n")
            } else {
                format!("{source}\\input{{extra}}\n")
            };
            std::fs::write(&root, source).unwrap();
            let project = ProjectSources::capture(&root);
            let before = if displayed {
                "(./palette.sty)\n(./extra.tex)"
            } else {
                "(./palette.sty)"
            };
            let after = if displayed { "" } else { "(./extra.tex)" };
            let log = log(call, false, before, after);
            let (diagnostics, loaded) = parse_with_project_sources(&root, &log, &project);
            let uri = Url::from_file_path(&root).unwrap();
            let diagnostic = &diagnostics[&uri][0];
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-color".into())),
                "{displayed}: {diagnostic:?}"
            );
            let sources: BTreeSet<Url> =
                serde_json::from_value(diagnostic.data.as_ref().unwrap()["macroSources"].clone())
                    .unwrap();
            let extra = Url::from_file_path(extra).unwrap();
            assert!(loaded.contains(&extra));
            assert!(!sources.contains(&extra));
            assert!(sources.contains(&uri));
            assert_eq!(sources.len(), 2);
        }
    }

    #[test]
    fn a_paused_parent_cannot_lend_future_definitions_to_a_child_error() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main.tex");
        let class = temp.path().join("local.cls");
        let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
        std::fs::write(&root, r"\documentclass{local}\newcommand{\bridge}{}").unwrap();
        std::fs::write(&class, call).unwrap();
        let project = ProjectSources::capture(&root);
        let log = format!("(./main.tex\n(./local.cls\n./local.cls:1: Package xcolor Error: Undefined color `accnet'.\n{HELP}l.1 {call}\n)\n)\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        let diagnostic = &result[&Url::from_file_path(class).unwrap()][0];
        assert!(
            diagnostic.code.is_none() && diagnostic.data.is_none(),
            "{diagnostic:?}"
        );
    }

    #[test]
    fn paused_parent_declarations_invalidate_existing_signatures_and_alias_targets() {
        for model in [false, true] {
            for (parent, prefix, precise) in [
                (r"\newenvironment{bridge}[3]{}{}", r"\bridge", false),
                (r"\newenvironment{done}[3]{}{}", r"\bridge", false),
                (r"\newenvironment{unrelated}[3]{}{}", r"\bridge", true),
                (r"\renewcommand{\bridge}[3]{}", r"\bridge", false),
                (r"\renewcommand{\done}[3]{}", r"\bridge", false),
                (r"\def\relax#1#2#3{}", r"\relax", false),
                (r"\renewcommand{\textcolor}[2]{}", "", false),
                (r"\newcommand{\unrelated}[3]{}", r"\bridge", true),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("main.tex");
                let child = temp.path().join("content.tex");
                let palette = temp.path().join("palette.sty");
                let call = format!(
                    "{prefix}{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                std::fs::write(
                    &root,
                    format!("\\usepackage{{palette}}{parent}\\input{{content}}"),
                )
                .unwrap();
                std::fs::write(&palette, r"\newcommand{\done}{}\def\bridge{\done}").unwrap();
                std::fs::write(&child, &call).unwrap();
                let project = ProjectSources::capture(&root);
                let message = if model {
                    "Undefined color model `rbg'."
                } else {
                    "Undefined color `accnet'."
                };
                let log = format!("(./main.tex\n(./palette.sty)\n(./content.tex\n./content.tex:1: Package xcolor Error: {message}\n{HELP}l.1 {call}\n)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let diagnostic = &result[&Url::from_file_path(&child).unwrap()][0];
                assert_eq!(
                    diagnostic.code.is_some(),
                    precise,
                    "{parent}: {diagnostic:?}"
                );
                if precise {
                    let data = diagnostic.data.as_ref().unwrap();
                    assert_eq!(
                        data["macroPausedSources"],
                        serde_json::json!([Url::from_file_path(&root).unwrap()])
                    );
                    // A changed paused source must also invalidate the snapshot.
                    std::fs::write(&root, "changed parent").unwrap();
                    let result = parse_with_project_sources(&root, &log, &project).0;
                    assert!(result[&Url::from_file_path(&child).unwrap()][0]
                        .code
                        .is_none());
                }
            }
        }
    }

    #[test]
    fn dynamically_loaded_children_cannot_borrow_an_incomplete_signature_set() {
        for model in [false, true] {
            let call = if model {
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
            } else {
                r"\bridge\textcolor{accnet}{MarkerUnique}"
            };
            let (temp, root, _) = fixture(r"\newcommand{\bridge}{}", "");
            let child = temp.path().join("dynamic.tex");
            std::fs::write(
                &root,
                r"\usepackage{palette}\def\filename{dynamic}\input{\filename}",
            )
            .unwrap();
            std::fs::write(&child, format!("\\renewcommand{{\\bridge}}[3]{{}}\n{call}")).unwrap();
            let project = ProjectSources::capture(&root);
            assert!(!project
                .documents
                .contains_key(&Url::from_file_path(&child).unwrap()));
            let message = if model {
                "Undefined color model `rbg'."
            } else {
                "Undefined color `accnet'."
            };
            let log = format!("(./main.tex\n(./palette.sty)\n(./dynamic.tex\n./dynamic.tex:2: Package xcolor Error: {message}\n{HELP}l.2 {call}\n)\n)\n");
            let result = parse_with_project_sources(&root, &log, &project).0;
            let diagnostic = &result[&Url::from_file_path(child).unwrap()][0];
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{diagnostic:?}"
            );
        }
    }

    #[test]
    fn dynamic_loads_can_change_signatures_after_returning_to_the_saved_parent() {
        for loader in [
            r"\input{\filename}",
            r"\InputIfFileExists{\filename}{}{}",
            r"\include{\filename}",
            r"\subfile{\filename}",
            r"\usepackage{palette,\filename}",
            r"\LoadClass{\filename}",
        ] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, _) = fixture(r"\newcommand{\bridge}{}", call);
            std::fs::write(&root, format!("\\usepackage{{palette}}{loader}\n{call}\n")).unwrap();
            std::fs::write(
                temp.path().join("dynamic.tex"),
                r"\renewcommand{\bridge}[3]{}",
            )
            .unwrap();
            let project = ProjectSources::capture(&root);
            let diagnostic = error(
                &root,
                &log(call, false, "(./palette.sty)\n(./dynamic.tex)", ""),
                &project,
            );
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{loader}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn changed_or_deleted_dependency_invalidates_precise_color_attribution() {
        for model in [false, true] {
            for deleted in [false, true] {
                let call = format!(
                    r"\bridge{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (temp, root, _) = fixture(r"\def\bridge{}", &call);
                let project = ProjectSources::capture(&root);
                if deleted {
                    std::fs::remove_file(temp.path().join("palette.sty")).unwrap();
                } else {
                    std::fs::write(
                        temp.path().join("palette.sty"),
                        r"\newcommand{\bridge}[3]{}",
                    )
                    .unwrap();
                }
                let diagnostic = error(&root, &log(&call, model, "(./palette.sty)", ""), &project);
                assert!(
                    diagnostic.code.is_none(),
                    "{model}/{deleted}: {diagnostic:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod indirect_file_error_tests {
    use super::*;

    #[test]
    fn indirect_input_errors_use_only_loaded_unchanged_alias_definitions() {
        for call in [
            r"\readpart{chapte}",
            r"\csuse{readpart}{chapte}",
            r"\csname readpart\endcsname{chapte}",
        ] {
            for case in ["loaded", "not-loaded", "changed", "shadowed"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path().join("main.tex");
                let package = directory.path().join("local.sty");
                std::fs::write(&root, format!("\\usepackage{{local}}\n{call}\n")).unwrap();
                std::fs::write(&package, r"\def\readpart{\input}").unwrap();
                if case == "shadowed" {
                    std::fs::write(
                        &root,
                        format!("\\usepackage{{local}}\\def\\input#1{{}}\n{call}\n"),
                    )
                    .unwrap();
                }
                let project = ProjectSources::capture(&root);
                if case == "changed" {
                    std::fs::write(&package, r"\def\readpart#1{}").unwrap();
                }
                let loaded = if case == "not-loaded" {
                    ""
                } else {
                    "(./local.sty)"
                };
                let log = format!("(./main.tex\n{loaded}\n! LaTeX Error: File `chapte.tex' not found.\n./main.tex:2: Emergency stop.\n<read *>\nl.2 {call}\n)");
                let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                let diagnostic = diagnostics[&Url::from_file_path(&root).unwrap()]
                    .iter()
                    .find(|d| d.message.contains("not found"))
                    .unwrap();
                assert_eq!(
                    diagnostic.code
                        == Some(lsp_types::NumberOrString::String("missing-input".into())),
                    case == "loaded",
                    "{case}/{call}: {diagnostic:?}"
                );
                if case == "loaded" {
                    assert_eq!(
                        diagnostic.data.as_ref().unwrap()["macroSources"]
                            .as_array()
                            .unwrap()
                            .len(),
                        2
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod alias_load_snapshot_tests {
    use super::*;

    #[test]
    fn alias_declarations_do_not_mask_dynamic_or_expanded_input_definitions() {
        for (definition, dynamic) in [
            (r"\def\reader{\input}", false),
            (r"\def\reader{\include}", false),
            (r"\def\reader{\subfile}", true),
            (r"\def\reader{\InputIfFileExists}", true),
            (r"\def\reader{\usepackage}", false),
            (r"\def\reader{\RequirePackage}", false),
            (r"\def\reader{\RequirePackageWithOptions}", false),
            (r"\def\reader{\documentclass}", false),
            (r"\def\reader{\LoadClass}", false),
            (r"\def\reader{\LoadClassWithOptions}", false),
            (r"\def\reader{\documentclass}\reader{\filename}", true),
            (r"\edef\reader{\LoadClass}", true),
            (r"\newcommand{\reader}[1]{\LoadClass{#1}}", true),
            (r"\edef\reader{\usepackage}", true),
            (r"\xdef\reader{\RequirePackage}", true),
            (r"\newcommand{\reader}[1]{\usepackage{#1}}", true),
            (r"\csuse{usepackage}{\filename}", true),
            (r"\csname RequirePackage\endcsname{known,\filename}", true),
            (r"\def\reader{\usepackage}\reader{\filename}", true),
            (
                r"\def\reader{\RequirePackage}\reader{known,\filename}",
                true,
            ),
            (r"\csuse{input}{\filename}", true),
            (r"\csname input\endcsname{\filename}", true),
            (r"\def\reader{\input}\reader{\filename}", true),
            (r"\def\reader{\include}\reader{\filename}", true),
            (r"\gdef\reader{\input}", false),
            (r"\newcommand{\reader}{\input}", false),
            (r"\newcommand{\reader}[0]{\input}", false),
            (r"\NewDocumentCommand{\reader}{}{\input}", false),
            (r"\csdef{reader}{\input}", false),
            (r"\def\reader#1{\input}", true),
            (r"\newcommand{\reader}[1]{\input}", true),
            (r"\NewDocumentCommand{\reader}{m}{\input}", true),
            (r"\edef\reader{\input}", true),
            (r"\xdef\reader{\input}", true),
            (r"\csedef{reader}{\input}", true),
            (r"\csxdef{reader}{\input}", true),
            (r"\def\reader{\relax\input}", true),
            (r"\def\reader{\input{\filename}}", true),
            (r"\newcommand{\reader}[1]{\input{#1}}", true),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("main.tex");
            std::fs::write(&root, definition).unwrap();
            let snapshot = ProjectSources::capture(&root);
            assert_eq!(
                snapshot
                    .dynamic_sources
                    .contains(&Url::from_file_path(&root).unwrap()),
                dynamic,
                "{definition}; alias tokens: {:?}; commands: {:?}",
                crate::symbols::commands::file_load_alias_body_positions(
                    definition,
                    &snapshot.documents[&Url::from_file_path(&root).unwrap()].syntax
                ),
                snapshot.documents[&Url::from_file_path(&root).unwrap()]
                    .syntax
                    .commands
            );
        }
    }
}

#[cfg(test)]
mod indirect_include_warning_tests {
    use super::*;

    #[test]
    fn indirect_chapter_warnings_use_loaded_unchanged_aliases_and_record_their_scope() {
        for call in [
            r"\readpart{chapte}",
            r"\csuse{readpart}{chapte}",
            r"\csname readpart\endcsname{chapte}",
        ] {
            for case in ["loaded", "not-loaded", "changed", "shadowed"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path().join("main.tex");
                let package = directory.path().join("local.sty");
                let source = format!("\\usepackage{{local}}\n{call}\n");
                std::fs::write(
                    &root,
                    if case == "shadowed" {
                        format!("\\usepackage{{local}}\\def\\include#1{{}}\n{call}\n")
                    } else {
                        source
                    },
                )
                .unwrap();
                std::fs::write(&package, r"\newcommand{\readpart}{\include}").unwrap();
                let project = ProjectSources::capture(&root);
                if case == "changed" {
                    std::fs::write(&package, r"\def\readpart#1{}").unwrap();
                }
                let loaded = if case == "not-loaded" {
                    ""
                } else {
                    "(./local.sty)"
                };
                let log = format!("(./main.tex\n{loaded}\nNo file main.aux.\nNo file chapte.aux.\nNo file chapte.tex.\n)");
                let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                let warnings: Vec<_> = diagnostics
                    .values()
                    .flatten()
                    .filter(|d| d.code == Some(NumberOrString::String("missing-include".into())))
                    .collect();
                assert_eq!(
                    warnings.len(),
                    usize::from(case == "loaded"),
                    "{case}/{call}: {warnings:?}"
                );
                if case == "loaded" {
                    let data = warnings[0].data.as_ref().unwrap();
                    assert_eq!(data["file"], "chapte.tex");
                    assert_eq!(data["macroSources"].as_array().unwrap().len(), 2);
                    assert_eq!(warnings[0].severity, Some(DiagnosticSeverity::WARNING));
                }
            }
        }
    }

    #[test]
    fn paused_parent_aliases_do_not_certify_indirect_chapter_warnings_in_children() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        std::fs::write(&root, r"\def\readpart{\include}\input{child}").unwrap();
        std::fs::write(directory.path().join("child.tex"), r"\readpart{chapte}").unwrap();
        let project = ProjectSources::capture(&root);
        let log = "(./main.tex\n(./child.tex\nNo file chapte.tex.\n)\n)";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        assert!(
            !diagnostics
                .values()
                .flatten()
                .any(|d| d.code == Some(NumberOrString::String("missing-include".into()))),
            "{diagnostics:?}"
        );
    }
}

#[cfg(test)]
mod indirect_package_scope_tests {
    use super::*;

    #[test]
    fn missing_packages_use_only_loaded_unchanged_indirect_dependencies() {
        for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
            for call in [
                r"\loadpkg",
                r"\csuse{loadpkg}",
                r"\csname loadpkg\endcsname",
            ] {
                for case in ["loaded", "not-loaded", "changed", "shadowed", "dynamic"] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("local.sty");
                    let source = format!(
                        "\\csname usepackage\\endcsname{{local}}\n{call}{{fixtuer}}\\relax\n"
                    );
                    std::fs::write(&root, &source).unwrap();
                    let declaration = format!("\\def\\loadpkg{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    let root_uri = Url::from_file_path(&root).unwrap();
                    let package_uri = Url::from_file_path(&package).unwrap();
                    // Redefining usepackage in the very package discovered
                    // through it makes the dependency graph oscillate. That
                    // file must not enter the saved signature scope.
                    assert_eq!(
                        project.documents.contains_key(&package_uri),
                        case != "shadowed" || loader != "usepackage",
                        "{case}/{loader}/{call}"
                    );
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadpkg#1{}").unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./local.sty)"
                    };
                    let log = format!("(./main.tex\n{loaded}\n! LaTeX Error: File `fixtuer.sty' not found.\n! Emergency stop.\n<read *>\nl.2 {call}{{fixtuer}}\\relax\n");
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = diagnostics[&root_uri]
                        .iter()
                        .find(|d| d.message.contains("not found"))
                        .unwrap();
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded")
                            .then(|| NumberOrString::String("missing-package".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(package_uri)));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parent_declarations_cannot_certify_a_child_package_alias() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let child = directory.path().join("content.tex");
        std::fs::write(&root, r"\def\loadpkg{\usepackage}\input{content}").unwrap();
        std::fs::write(&child, "\\loadpkg{fixtuer}\\relax\n").unwrap();
        let project = ProjectSources::capture(&root);
        let log = "(./main.tex (./content.tex\n! LaTeX Error: File `fixtuer.sty' not found.\n! Emergency stop.\n<read *>\nl.1 \\loadpkg{fixtuer}\\relax\n";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        let diagnostic = diagnostics[&Url::from_file_path(&child).unwrap()]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{diagnostic:?}");
    }
}
#[cfg(test)]
mod indirect_class_scope_tests {
    use super::*;

    #[test]
    fn class_errors_use_only_loaded_unchanged_alias_definitions() {
        for loader in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
            for call in [
                r"\loadclass",
                r"\csuse{loadclass}",
                r"\csname loadclass\endcsname",
            ] {
                for case in ["loaded", "not-loaded", "changed", "shadowed", "dynamic"] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("definitions.sty");
                    let source =
                        format!("\\RequirePackage{{definitions}}\n{call}{{fixtuer}}\\relax\n");
                    std::fs::write(&root, &source).unwrap();
                    let declaration = format!("\\def\\loadclass{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    let package_uri = Url::from_file_path(&package).unwrap();
                    assert!(project.documents.contains_key(&package_uri));
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadclass#1{}").unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./definitions.sty)"
                    };
                    let log=format!("(./main.tex\n{loaded}\n! LaTeX Error: File `fixtuer.cls' not found.\n! Emergency stop.\n<read *>\nl.2 {call}{{fixtuer}}\\relax\n");
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = diagnostics[&Url::from_file_path(&root).unwrap()]
                        .iter()
                        .find(|d| d.message.contains("not found"))
                        .unwrap();
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded").then(|| NumberOrString::String("missing-class".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(package_uri)));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parent_declarations_do_not_certify_child_class_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let child = directory.path().join("content.tex");
        std::fs::write(&root, r"\def\loadclass{\documentclass}\input{content}").unwrap();
        std::fs::write(&child, "\\loadclass{fixtuer}\\relax\n").unwrap();
        let project = ProjectSources::capture(&root);
        let log="(./main.tex (./content.tex\n! LaTeX Error: File `fixtuer.cls' not found.\n! Emergency stop.\n<read *>\nl.1 \\loadclass{fixtuer}\\relax\n";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        let diagnostic = diagnostics[&Url::from_file_path(&child).unwrap()]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{diagnostic:?}");
    }
}

#[cfg(test)]
mod indirect_option_clash_scope_tests {
    use super::*;

    fn error(echo: &str, line: usize) -> String {
        format!("! LaTeX Error: Option clash for package local.\n\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \nl.{line} {echo}\n                                                  \nThe package local has already been loaded with options:\n  [draft]\nThere has now been an attempt to load it with options\n  [final]\nAdding the global options:\n")
    }

    #[test]
    fn option_clashes_use_only_loaded_and_unchanged_alias_definitions() {
        for loader in ["usepackage", "RequirePackage"] {
            for call in [
                r"\loadpkg",
                r"\csuse{loadpkg}",
                r"\csname loadpkg\endcsname",
            ] {
                for case in [
                    "loaded",
                    "not-loaded",
                    "changed",
                    "root-changed",
                    "shadowed",
                    "dynamic",
                ] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("definitions.sty");
                    let echo = format!("{call}[final]{{local}}\\relax");
                    std::fs::write(&root, format!("\\RequirePackage{{definitions}}\n{echo}\n"))
                        .unwrap();
                    let declaration = format!("\\def\\loadpkg{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadpkg#1{}").unwrap();
                    }
                    if case == "root-changed" {
                        std::fs::write(
                            &root,
                            format!("\\RequirePackage{{definitions}}\n{echo}\n\\relax\n"),
                        )
                        .unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./definitions.sty)"
                    };
                    let log = format!("(./main.tex\n{loaded}\n{}", error(&echo, 2));
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = &diagnostics[&Url::from_file_path(&root).unwrap()][0];
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded")
                            .then(|| NumberOrString::String("package-option-clash".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert_eq!(data["loadedOptions"], "draft");
                        assert_eq!(data["requestedOptions"], "final");
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(Url::from_file_path(&package).unwrap())));
                    } else {
                        assert!(diagnostic
                            .data
                            .as_ref()
                            .is_none_or(|data| data.get("loadedOptions").is_none()));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parents_only_allow_independent_child_loader_aliases() {
        for conflict in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("main.tex");
            let child = directory.path().join("content.tex");
            std::fs::write(
                &root,
                if conflict {
                    r"\def\loadpkg{\usepackage}\input{content}"
                } else {
                    r"\input{content}"
                },
            )
            .unwrap();
            let echo = r"\loadpkg[final]{local}\relax";
            std::fs::write(&child, format!("\\def\\loadpkg{{\\usepackage}}\n{echo}\n")).unwrap();
            let project = ProjectSources::capture(&root);
            let log = format!("(./main.tex (./content.tex\n{}", error(echo, 2));
            let diagnostics = parse_with_project_sources(&root, &log, &project).0;
            let diagnostic = &diagnostics[&Url::from_file_path(&child).unwrap()][0];
            assert_eq!(
                diagnostic.code,
                (!conflict).then(|| NumberOrString::String("package-option-clash".into())),
                "{conflict}: {diagnostic:?}"
            );
            if !conflict {
                assert_eq!(
                    diagnostic.data.as_ref().unwrap()["macroPausedSources"],
                    serde_json::json!([Url::from_file_path(&root).unwrap()])
                );
            }
        }
    }
}
#[cfg(test)]
mod indirect_bibliography_error_scopes {
    use super::*;

    fn call(form: &str, command: &str) -> String {
        match form {
            "named" => format!("\\csuse{{{command}}}"),
            "literal" => format!("\\csname {command}\\endcsname"),
            "alias" => r"\loadbib".into(),
            "named-alias" => r"\csuse{loadbib}".into(),
            "literal-alias" => r"\csname loadbib\endcsname".into(),
            _ => panic!(),
        }
    }

    fn failure(main: &Path, command: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
        let (kind, name) = if command == "bibliographystyle" {
            ("style", "local,styel.bst")
        } else {
            ("database", "reffs.bib")
        };
        parse_bibliography(
            main,
            &format!("I couldn't open {kind} file {name}\n---line 3 of file main.aux\n"),
        )
    }

    #[test]
    fn resource_errors_use_confirmed_alias_providers_and_record_the_full_scope() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let provider = directory.path().join("definitions.sty");
                let name = if command == "bibliographystyle" {
                    "local,styel"
                } else {
                    "re ffs.bib"
                };
                let source = format!(
                    "% 😀\r\n\\RequirePackage{{definitions}}\n{}{{% before\r\n{name}% tail\r\n}}",
                    call(form, command)
                );
                std::fs::write(&main, &source).unwrap();
                std::fs::write(&provider, format!("\\def\\loadbib{{\\{command}}}")).unwrap();
                let main_uri = Url::from_file_path(&main).unwrap();
                let provider_uri = Url::from_file_path(&provider).unwrap();
                let sources = BTreeSet::from([main_uri.clone(), provider_uri]);
                let mut diagnostics = failure(&main, command);
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                let diagnostic = &diagnostics[&main_uri][0];
                let start = source.find(name).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source.clone()).range(start, start + name.len()),
                    "{command}/{form}"
                );
                assert!(
                    !unresolved_location(diagnostic),
                    "{command}/{form}: {diagnostic:?}"
                );
                assert_eq!(
                    serde_json::from_value::<BTreeSet<Url>>(
                        diagnostic.data.as_ref().unwrap()["macroSources"].clone()
                    )
                    .unwrap(),
                    sources
                );
            }
        }
    }

    #[test]
    fn unread_alias_providers_do_not_certify_bibliography_failures() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let provider = directory.path().join("definitions.sty");
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            std::fs::write(&main, format!("\\loadbib{{{name}}}")).unwrap();
            std::fs::write(provider, format!("\\def\\loadbib{{\\{command}}}")).unwrap();
            let main_uri = Url::from_file_path(&main).unwrap();
            let mut diagnostics = failure(&main, command);
            locate_missing_bibliography_resources(
                &BTreeSet::from([main_uri.clone()]),
                &mut diagnostics,
            );
            assert!(unresolved_location(&diagnostics[&main_uri][0]));
        }
    }

    #[test]
    fn inactive_uncertain_and_ambiguous_resource_calls_keep_unresolved_locations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let sources = BTreeSet::from([main_uri.clone()]);
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            for body in [
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{\\loadbib{{{name}}}}}"),
                format!("\\newcommand{{\\later}}{{\\loadbib{{{name}}}}}"),
                format!("\\endinput\n\\loadbib{{{name}}}"),
                r"\loadbib{\chosen}".into(),
                format!("\\renewcommand{{\\{command}}}[1]{{}}\\loadbib{{{name}}}"),
                format!("\\loadbib{{{name}}}\\{command}{{{name}}}"),
                format!("\\loadbib{{{name}}}\\loadbib{{\\chosen}}"),
                // The old direct-only lookup must not preserve this unused
                // declaration as a unique source occurrence.
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{\\{command}{{{name}}}}}"),
            ] {
                let source = format!("\\def\\loadbib{{\\{command}}}\n{body}");
                std::fs::write(&main, source).unwrap();
                let mut diagnostics = failure(&main, command);
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                assert!(
                    unresolved_location(&diagnostics[&main_uri][0]),
                    "{command}/{body}: {diagnostics:?}"
                );
            }
        }
    }

    #[test]
    fn unused_alias_bodies_do_not_hide_direct_bibliography_declarations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            let source = format!("\\def\\unused{{\\{command}}}\n\\{command}{{{name}}}");
            std::fs::write(&main, &source).unwrap();
            let mut diagnostics = failure(&main, command);
            locate_missing_bibliography_resources(
                &BTreeSet::from([main_uri.clone()]),
                &mut diagnostics,
            );
            let item = &diagnostics[&main_uri][0];
            assert!(!unresolved_location(item), "{command}: {item:?}");
            let start = source.rfind(name).unwrap();
            assert_eq!(
                item.range,
                Text::new(source.clone()).range(start, start + name.len())
            );
            assert!(item.data.as_ref().unwrap().get("macroSources").is_none());
        }
    }

    #[test]
    fn incomplete_source_scopes_revoke_old_direct_bibliography_locations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let other = directory.path().join("other.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            std::fs::write(&main, format!("\\{command}{{{name}}}")).unwrap();
            let sources = BTreeSet::from([main_uri.clone(), Url::from_file_path(&other).unwrap()]);
            for oversized in [false, true] {
                if oversized {
                    std::fs::write(&other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
                }
                let mut diagnostics = failure(&main, command);
                assert!(!unresolved_location(&diagnostics[&main_uri][0]));
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                assert!(unresolved_location(&diagnostics[&main_uri][0]));
            }
        }
    }
}
