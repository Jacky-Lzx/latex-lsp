import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { extname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import sidebar from '../docs/.vitepress/sidebar.mjs'

const docs = fileURLToPath(new URL('../docs/', import.meta.url))
const output = join(docs, '.vitepress/dist')
const base = `/${(process.env.DOCS_BASE || '').replace(/^\/+|\/+$/g, '')}/`.replace('//', '/')
const origin = 'https://docs.example'
const failures = new Set()

function navigationPages(items) {
  return items.flatMap((item) => [item.link, ...navigationPages(item.items || [])]).filter(Boolean)
}

const navigation = new Set(navigationPages(sidebar))
for (const file of readdirSync(docs).filter((file) => file.endsWith('.md') && file !== 'index.md')) {
  if (!navigation.has(`/${file.slice(0, -3)}`)) failures.add(`Missing sidebar entry: ${file}`)
}

function htmlFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? htmlFiles(path) : entry.name.endsWith('.html') ? [path] : []
  })
}

function decodeHtml(value) {
  return value.replace(/&(?:amp|quot|apos|lt|gt|#\d+|#x[\da-f]+);/gi, (entity) => {
    const named = { '&amp;': '&', '&quot;': '"', '&apos;': "'", '&lt;': '<', '&gt;': '>' }
    if (entity in named) return named[entity]
    return String.fromCodePoint(entity[2].toLowerCase() === 'x'
      ? parseInt(entity.slice(3, -1), 16)
      : parseInt(entity.slice(2, -1), 10))
  })
}

const files = htmlFiles(output)
const anchors = new Map(files.map((file) => [file, new Set(
  [...readFileSync(file, 'utf8').matchAll(/\bid="([^"]+)"/g)].map((match) => decodeHtml(match[1])),
)]))
let links = 0

for (const file of files) {
  const source = relative(output, file).replaceAll('\\', '/')
  const pageUrl = new URL(`${base}${source}`, origin)
  const html = readFileSync(file, 'utf8')
  for (const match of html.matchAll(/\b(?:href|src)="([^"]+)"/g)) {
    const href = decodeHtml(match[1])
    const url = new URL(href, pageUrl)
    if (url.origin !== origin) continue
    links++
    if (!url.pathname.startsWith(base)) {
      failures.add(`${source}: link escapes site base: ${href}`)
      continue
    }
    let target = join(output, decodeURIComponent(url.pathname.slice(base.length)))
    if (existsSync(target) && statSync(target).isDirectory()) target = join(target, 'index.html')
    else if (!extname(target)) target += '.html'
    if (!existsSync(target)) {
      failures.add(`${source}: missing target: ${href}`)
    } else if (url.hash && anchors.has(target) && !anchors.get(target).has(decodeURIComponent(url.hash.slice(1)))) {
      failures.add(`${source}: missing heading: ${href}`)
    }
  }
}

if (failures.size) {
  console.error([...failures].join('\n'))
  process.exitCode = 1
} else {
  console.log(`Documentation checks passed: ${files.length} HTML pages, ${links} internal links/assets, complete sidebar coverage.`)
}
