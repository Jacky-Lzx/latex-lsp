# TeX distributions and packages

After the server is started, `kpsewhich --expand-path=$TEXMFDBS` is called in the background and the `ls-R` file name database of the returned directory is read; distribution discovery and file monitor registration will not block document opening, editing or forward search. Before the database is ready, related completion and file jumps first use the files in the project and built-in metadata. Completions and file jumps for `usepackage`, `RequirePackage`, document classes, and `bibliographystyle` will look for releases; project files with the same name take precedence. Files that no longer exist after being deleted or moved are not returned as results. Release source files are not automatically bulleted, renamed, or diagnostic scoped.

```json
{
  "latex": {
    "distribution": {
      "enabled": true,
      "executable": "kpsewhich",
      "roots": []
    }
  }
}
```

`roots` is an additional TeX tree root directory. Each directory needs to contain `ls-R`, taking precedence over automatically discovered directories. In-project functionality can still be used without `kpsewhich` or database. The discovery process runs for a maximum of two seconds; reads a maximum of 64 directories, 32 MiB per database, and 100,000 filenames in total. Modify the configuration or restart the server to refresh the database.

`assets/packages.json` released with the server provides extensible package description and command parameter metadata. It currently covers common commands for amsmath, mathtools, amsfonts, amssymb, booktabs, geometry, and graphicx, as well as package instructions for hyperref, fontspec, tikz, pgfplots, and biblatex. The dependencies of static package loading and metadata declaration in the project source code determine command completion, signature help and hover documents; user declarations take precedence. A link to the texdoc.org documentation is provided on hover.

This is a curated set of metadata and does not execute packages or automatically analyze all macro definitions in a distribution. Dynamic loading, conditional loading, and implicit package dependencies within document classes may require explicit loading or supplemental metadata. Any search paths for files within the `\input` distribution are not currently provided by this index.
