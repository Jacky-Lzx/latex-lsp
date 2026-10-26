# Cleaning build artifacts

[Return to Build guide](build.md)

Called via `workspace/executeCommand`:

- `latex.cleanAuxiliary`: Clean known auxiliary files in the current root document, jobName and product directories, preserving PDF and SyncTeX.
- `latex.cleanArtifacts`: Additional cleanup of PDF, `.synctex` and `.synctex.gz` for this task.

The first parameter can be a document URI, or an object containing `uri` and an optional Boolean value `dryRun`. You can preview first:

```json
{"command":"latex.cleanAuxiliary","arguments":[{"uri":"file:///project/main.tex","dryRun":true}]}
```

Returns `rootUri`, `outputDirectory`, `artifacts`, `dryRun`, candidate `files`, actual `removed`, `skipped` with reason, and file-by-file `errors`. Preview does not modify files or build status; the actual cleanup regenerates and verifies the manifest. I/O failures may leave partial files, and deleted paths may still be returned in full.

Only common auxiliary suffixes for the current task (aux, bbl, blg, bcf, run.xml, fdb_latexmk, fls, log, directory/navigation and index files) are processed. The complete `.fls` record can supplement the `.aux` in the subsection of the product catalog; explicitly configured named indexes are also included in the manifest. The entire directory will not be scanned or external cleanup programs performed. `.bib`, `.ist`, source code, known dependencies, open buffers, directories, and symbolic links are protected; on Unix, hard links to source code are also protected.

Cleanup is denied when there is an active compilation in the current job or in the same product directory; also when sharing PDF or index output with another known job. Please stop compiling first, or set a different jobName/product directory for the project. If the directory does not exist, an empty list is returned and the directory is not created. Review output directory bindings, file bindings, metadata, and content before execution; do not initiate deletion when schedule expires. Any further changes during execution will stop and an error will be reported. The file system does not provide transaction rollback for concurrent changes to external processes.

Active compilation retains both the original physical artifact location and the directory path passed to the compiler; after the directory symlink is redirected, cleanup and PDF/named index building from another root must still pass the conflict check of the current binding. When `..` in the path traverses symbolic links, causing the actual directory and the lexical path to point to different locations, cleanup will reject and require the use of the direct directory; similar file paths in the recorder will be skipped. The manifest also retains the binding of the original path, preventing it from being deleted by the collapsed path if it changes.

After actual deletion, the old compilation status and compilation diagnostics of the current task are cleared, and the client is updated through the push/pull mechanism; the auxiliary cleaning retains the input history of the PDF, and the complete cleaning also invalidates the history. Static editing diagnostics continue to work. The maximum number of candidates in a single list is 4096, the total content verification budget is 128 MiB, and the maximum recorder size is 4 MiB/32768 lines; when the limit is exceeded, incomplete recorders are rejected or ignored to avoid infinite traversal.
