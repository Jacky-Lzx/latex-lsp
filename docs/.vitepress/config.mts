import { defineConfig } from 'vitepress'
import { dirname, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { slug } from 'github-slugger'
import sidebar from './sidebar.mjs'

const docsRoot = fileURLToPath(new URL('../', import.meta.url))
const repositoryRoot = resolve(docsRoot, '..')
const repository = process.env.GITHUB_REPOSITORY || 'Jacky-Lzx/texquill'
const repositoryUrl = `https://github.com/${repository}`
const base = `/${(process.env.DOCS_BASE || '').replace(/^\/+|\/+$/g, '')}/`.replace('//', '/')

export default defineConfig({
  title: 'TeXquill',
  description: 'A Rust language server for LaTeX. Completion, navigation, safe refactoring, builds, and diagnostics in your editor.',
  lang: 'en-US',
  base,
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}favicon.svg` }],
    ['meta', { name: 'theme-color', content: '#137c72' }],
  ],
  lastUpdated: true,
  markdown: {
    // Preserve heading links used by the existing GitHub Markdown documentation.
    anchor: { slugify: slug },
    // Keep TeX examples literal: do not interpret $...$ as rendered mathematics.
    languages: ['latex', 'bibtex', 'lua', 'json', 'bash', 'rust', 'toml'],
    config(md) {
      // GitHub can resolve links outside docs/; the static website cannot.
      // Rewrite only those links, leaving the Markdown sources usable in both places.
      md.core.ruler.after('inline', 'documentation-tokens', (state) => {
        const source = state.env.path
        if (!source) return
        for (const block of state.tokens) {
          for (const token of block.children || []) {
            // TeX grouping such as {{figures/}} is not Vue interpolation.
            if (token.type === 'code_inline') token.attrSet('v-pre', '')
            if (token.type !== 'link_open') continue
            const href = token.attrGet('href')
            if (!href || /^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(href)) continue
            const path = href.split(/[?#]/, 1)[0]
            const suffix = href.slice(path.length)
            const target = resolve(dirname(source), decodeURI(path))
            const insideDocs = relative(docsRoot, target)
            if (insideDocs === '..' || insideDocs.startsWith(`..${sep}`)) {
              const repoPath = relative(repositoryRoot, target).split(sep).join('/')
              token.attrSet('href', `${repositoryUrl}/blob/main/${repoPath}${suffix}`)
            }
          }
        }
      })
    },
  },
  themeConfig: {
    logo: '/favicon.svg',
    nav: [
      { text: 'Guide', link: '/install' },
      { text: 'Demos', link: '/demos' },
      { text: 'Features', link: '/editing' },
      { text: 'Builds', link: '/build' },
      { text: 'Reference', link: '/project-settings' },
      { text: 'Development', link: '/development' },
    ],
    sidebar,
    search: { provider: 'local' },
    outline: { level: [2, 3], label: 'On this page' },
    editLink: {
      pattern: `${repositoryUrl}/edit/main/docs/:path`,
      text: 'Edit this page on GitHub',
    },
    socialLinks: [{ icon: 'github', link: repositoryUrl }],
    footer: {
      message: `<a href="${repositoryUrl}/blob/main/LICENSE">Released under the MIT License.</a>`,
      copyright: 'TeXquill contributors',
    },
  },
})
