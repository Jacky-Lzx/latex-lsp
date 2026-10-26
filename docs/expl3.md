# expl3 editing

Between `\ExplSyntaxOn` and `\ExplSyntaxOff`, the server uses `_` and `:` as part of the control sequence name. The complete declaration of `ProvidesExplPackage`, `ProvidesExplClass`, and `ProvidesExplFile` also turns on this mode; when the ordinary TeX grouping ends, the mode before entering the grouping is restored. Comments, verbatim code, and deferred mode switches in macro bodies do not change peripheral lexical state.

The following declarations of direct static targets are supported:

- `cs_new`, `cs_set`, `cs_gset` and their `protected` / `nopar` forms `:Npn`, `:Npx`, `:Npe`, `:Nn`, `:Nx`, `:Ne`; `cs_new_eq:NN`, `cs_set_eq:NN`, `cs_gset_eq:NN` Alias.
- `tl`, `str`, `int`, `seq`, `prop`, `bool`, `dim`, `skip`, `muskip`, `clist`, `fp`, `box`, `ior`, `iow`, `coffin` of `new:N` and `const:Nn`/`const:Nx`/`const:Ne`.

Project functions and variables offer full name completion, definitions, references, hover, cross-file renaming, document notation, and semantic highlighting. Completion also includes common expl3 base functions. Renaming retains parameter signatures such as `:nn` for functions, as well as `l_` / `g_` / `c_` ranges and type suffixes such as `_tl` for variables; the operation is rejected when the declaration is not unique or conflicts with an existing name.

Signature help are deduced from the project declaration or the name suffix of the known basic function, distinguishing group parameters such as `N` single token and `n`, as well as `T` / `F` branches. The explicit `:Npn` parameter text must be a plain `#1 ... #9` consistent with the name signature; delimited parameter text, `p` / `w` parameters are not speculative prompts.

This is static editing support: no expansion, arbitrary catcode rewriting, or conditional definitions are performed, no dynamic name `:c` is generated, and implicit declarations resulting from `cs_generate_variant:Nn` are not expanded. Complex partial masking and runtime declarations require manual checking; normal LaTeX mode still uses the original control word rules.
