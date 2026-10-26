const fs = require('node:fs/promises');
const path = require('node:path');

async function main() {
  const root = path.resolve(__dirname, '..');
  const assets = path.join(root, 'dist/preview/assets');
  const pdfjs = path.join(root, 'node_modules/pdfjs-dist');
  await fs.rm(assets, { recursive: true, force: true });
  await fs.cp(path.join(root, 'media/preview'), assets, { recursive: true });
  const vendor = path.join(assets, 'vendor');
  await fs.mkdir(vendor);
  for (const [source, target] of [
    ['legacy/build/pdf.min.mjs', 'pdf.mjs'],
    ['legacy/build/pdf.worker.min.mjs', 'pdf.worker.mjs'],
    ['legacy/web/pdf_viewer.mjs', 'pdf_viewer.mjs'],
    ['web/pdf_viewer.css', 'pdf_viewer.css'],
    ['LICENSE', 'LICENSE'],
  ]) {
    await fs.copyFile(path.join(pdfjs, source), path.join(vendor, target));
  }
  for (const directory of ['cmaps', 'standard_fonts', 'wasm', 'iccs', 'web/images']) {
    await fs.cp(path.join(pdfjs, directory), path.join(vendor, path.basename(directory)), { recursive: true });
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
