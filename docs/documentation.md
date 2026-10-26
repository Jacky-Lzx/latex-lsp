# Maintaining this website

The website uses VitePress and the Markdown files in `docs/`. Keep feature documentation here so GitHub readers and the website share the same source.

## Local development

Use Node.js 22 or later. From the repository root:

```sh
npm ci
npm run docs:dev
```

Open the local URL printed by VitePress. Changes to Markdown, navigation, and theme styles reload automatically. These dependencies are separate from the VS Code extension in `editors/vscode/`.

To build and inspect the production website:

```sh
npm run docs:build
npm run docs:preview
```

The build writes to `docs/.vitepress/dist/` and checks generated internal links, heading anchors, and sidebar coverage. Keep broken-link checking enabled.

## Content and navigation

- Edit `docs/index.md` for the homepage and `docs/README.md` for the documentation overview.
- Add new pages to `docs/.vitepress/sidebar.mjs`. Use a short navigation label when the page title is long.
- Configure navigation, local full-text search, repository links, and metadata in `docs/.vitepress/config.mts`.
- Keep relative `.md` links between documentation pages. Links outside `docs/` are converted to GitHub source links during rendering.
- Heading anchors follow GitHub's slug format, including titles beginning with dates.
- Put LaTeX commands and examples in inline code or fenced code blocks. Automatic math rendering is disabled so literal TeX syntax is preserved.
- Keep shared styles in `docs/.vitepress/theme/style.css` and static assets in `docs/public/`.

The current documentation is English. Add translated content and VitePress locale configuration together when translations are available.

## GitHub Pages

In the repository's **Settings → Pages**, select **GitHub Actions** as the build and deployment source. The `Documentation` workflow builds pull requests, and publishes successful builds on pushes to `main` or a manual workflow run on `main`.

The workflow reads the Pages base path from GitHub, so repository subpaths and custom domains use the same configuration. For a local subpath check, set `DOCS_BASE` explicitly:

```sh
DOCS_BASE=/latex-lsp/ npm run docs:build
DOCS_BASE=/latex-lsp/ npm run docs:preview
```

`GITHUB_REPOSITORY` selects the repository used by source and edit links in CI. Local builds default to the current Git remote, `Jacky-Lzx/latex-lsp`; override it when previewing a fork or a renamed repository. Generated files, dependencies, and caches are ignored by Git.
