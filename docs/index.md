---
layout: home
title: LaTeX language tools for your editor
hero:
  name: TeXquill
  text: Write LaTeX. Keep your flow.
  tagline: A language server written in Rust, with project-aware completion, navigation, refactoring, and build diagnostics. For Neovim, VS Code, and other LSP editors.
  actions:
    - theme: brand
      text: Get started
      link: /install
    - theme: alt
      text: Set up your editor
      link: /editors
    - theme: alt
      text: Browse the documentation
      link: /README
features:
  - title: Complete in context
    details: Commands, environments, package options, citations, and file paths, with signatures for built-in and user-defined macros.
    link: /completion
    linkText: Explore completion
  - title: Navigate your project
    details: Follow definitions and references across files. Find labels, bibliography entries, macros, and project symbols.
    link: /editing
    linkText: Explore editing features
  - title: Refactor with confidence
    details: Rename symbols across files, update references when files move, and apply quick fixes with ambiguity checks.
    link: /refactoring
    linkText: Explore refactoring
  - title: Build from your editor
    details: Run latexmk or a configured TeX pipeline, track build progress, cancel jobs, and inspect errors at their source.
    link: /build
    linkText: Configure builds
  - title: Work with your bibliography
    details: Search citations by author, title, or year, navigate BibTeX entries, and work with biblatex resources and relationships.
    link: /bibliography
    linkText: Explore bibliography support
  - title: Make it fit your project
    details: Configure formatting, diagnostics, build tools, and indexing with project settings and editor configuration.
    link: /project-settings
    linkText: Read the configuration guide
---

## See it in your editor

Find a paper by its title, preview its bibliography entry, and insert the citation key. Recorded in Neovim with Blink completion and the maintainer's configuration.

<DemoVideo name="completion" title="Search citations by title in Neovim" />

[Watch all eight demos](demos.md): navigation and rename, quick fixes, linked editing, macro documentation, formatting, builds, and forward/reverse PDF search.

## From source to your editor

Install the current stable Rust toolchain, then run these commands from the repository root:

```sh
cargo install --path . --locked
texquill --version
```

[Configure Neovim or VS Code](editors.md) to launch `texquill --stdio`. Editing features work without a TeX distribution. To produce PDFs, install the tools required by your [build configuration](build-configuration.md).

## Find the right guide

- **New to TeXquill?** Start with [installation](install.md), [editor setup](editors.md), and the [sample project](editors.md#sample-project).
- **Working on a paper?** Read about [completion](completion.md), [cross-references](labels.md), [citations](citation-search.md), and [building PDFs](build.md).
- **Tuning your setup?** See [project settings](project-settings.md), [formatting](formatting.md), and [diagnostics](static-diagnostics.md).
- **Contributing?** Explore the [architecture](development.md), [tests with real tools](testing.md), and [performance benchmarks](benchmarks.md).

TeXquill analyzes source statically and does not execute arbitrary TeX macro expansion. Individual feature guides describe their supported syntax and limits.
