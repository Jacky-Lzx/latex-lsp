# Indexing budgets and caches

Initial file reading and parsing are performed in a restricted background worker thread, and indexing is completed before the initial response is returned so that first navigation and completion use a consistent project snapshot. The Tokio execution thread will not be occupied during scanning; boundary checks on scanned directories and files are canceled after disconnection.

```json
{
  "latex": {
    "indexing": {
      "maxFiles": 1024,
      "maxFileSize": 2097152,
      "maxTotalBytes": 67108864,
      "maxDepth": 16,
      "completionLimit": 256,
      "exclude": ["target", "build", "node_modules", "vendor"]
    }
  }
}
```

These are server-level configurations and cannot be written to project `.texquill.json`. Hidden directories are skipped by default; `exclude` matches by directory name. Disk indexing is limited by the number of files, single file bytes, total bytes, and directory/dependency depth. Buffers opened by the editor can still be used.

`latex.indexingStatus` of `workspace/executeCommand` returns `files`, `truncated`, and `limits` without URI parameters. `truncated` lists the hit limits, returns `CompletionList` when the completion limit is reached or the index is truncated, and sets `isIncomplete: true`. `latex.reindex` Rebuilds the disk index in a background worker thread and returns to the same state, preserving unsaved buffers. Indexes will be automatically rebuilt after modifying the budget through configuration notifications.

Each document version caches its full semantic token results; only changed documents are regenerated. This cache is cleared when the corresponding document is reparsed across file semantic changes. The existing bounded LSP delta cache continues to be used to calculate the difference; the semantic result cache retains a total of up to 65536 tokens, and when the budget is insufficient, the results are returned directly without retaining additional copies. Currently, the entire document is still used as the syntax reparsing unit.

The same dependency traversal reuses the input interpretation of the root document; re-analyzes after loading new files, changing parsing types, and refreshing disk content. Project ownership gives priority to confirmed source code dependencies, and image dependencies are resolved based on actual calls. The macro definition text uses merge interval query, and the command name navigation directly enters its symbol analysis. When opening, modifying, and closing unsaved buffers, the content of this document is used to recalculate the corresponding context.

The flat outline and workspace symbol search of the old client construct and share the source code outline cache, and the hierarchical outline is reused when the cache already exists; the first hierarchical request directly returns new results, avoiding the need to copy the entire tree in order to create a cache. The cache is bound to the language role according to the document content; it is reconstructed after modifying, reopening, changing the parsing role, or moving the document. The background snapshot can continue to hold the old version, and the memory budget of the old version is returned after the last reference is released. Cached containers, names, and scopes are all from a single copy of the source code, and no project parsing results or file URIs are cached.

The outline cache retains a maximum of approximately 4 MiB of heap data across the entire process, estimated by vector capacity, name/description string capacity, and snapshot overhead; full results are still returned when exceeded, and only the additional cache is discarded. The workspace search traverses the borrowed syllabus, ranking it with a candidate queue of up to 256 entries, assigning protocol results only to entries that entered the candidate set; retaining the original stable ordering of match priorities, containers, and colocations. Overrun uncached synopses are released after each document is processed. The cache only uses memory and does not generate disk files.
