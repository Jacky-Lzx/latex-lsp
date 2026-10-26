# Citation search completion

In addition to the citation key prefix, you can also enter the author, title, year, or a combination of these information in the citation parameter. For example, `\cite{Knuth 1984}` can search for the author and year in the document preview, accept the candidate, and replace it with the corresponding real citation key.

Sorting considers exact prefixes, case-insensitive prefixes, substrings within a key, metadata keywords, subsequences of keys, and bounded spelling distance in order. The completion tag displays the citation key, and the details display the existing author, title, year preview, and source; client-side text filtering will retain the current query, preventing the editor from hiding author query candidates.

The search scope is consistent with the project literature scope. Normal citations, citation lists, single citations, and biblatex multi-group/volume number citations share this behavior, and Chinese and other Unicode metadata can also be retrieved. Field editing such as crossref in BibTeX databases continues to use citation key completion. Dynamic macros or annotated queries retain existing literal key processing without any text replacement.

`bibentry`, `defcitealias`, `entrydata`, `ifentryexists`, `ifentryinbib` and `ifentrycategory` have strict single citation keys, continue to use the original key completion and parameter boundary checking, and do not enable metadata retrieval.
