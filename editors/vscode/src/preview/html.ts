import { randomBytes } from 'node:crypto';
import * as vscode from 'vscode';

function attribute(value: string): string {
  return value.replace(/[&<>"']/g, character => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[character]!);
}

export function previewHtml(webview: vscode.Webview, assets: vscode.Uri): string {
  const resource = (name: string) => webview.asWebviewUri(vscode.Uri.joinPath(assets, name)).toString();
  const nonce = randomBytes(16).toString('hex');
  const source = webview.cspSource;
  const config = attribute(JSON.stringify({
    worker: resource('vendor/pdf.worker.mjs'),
    resources: `${resource('vendor')}/`,
  }));
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}' ${source} 'wasm-unsafe-eval'; style-src ${source} 'unsafe-inline'; img-src ${source} data: blob:; font-src ${source} data:; connect-src ${source}; worker-src ${source} blob:;">
  <meta name="texquill-pdf-config" content="${config}">
  <link rel="stylesheet" href="${attribute(resource('vendor/pdf_viewer.css'))}">
  <link rel="stylesheet" href="${attribute(resource('viewer.css'))}">
  <title>TeXquill PDF Preview</title>
</head>
<body>
  <header id="toolbar" role="toolbar" aria-label="PDF navigation">
    <div class="controls">
      <button id="previous" title="Previous page" aria-label="Previous page" disabled>‹</button>
      <label class="page-label" for="page">Page</label>
      <input id="page" type="number" min="1" value="1" aria-label="Page" disabled>
      <span id="page-count" aria-label="Page count">/ —</span>
      <button id="next" title="Next page" aria-label="Next page" disabled>›</button>
    </div>
    <div class="controls zoom-controls">
      <button id="zoom-out" title="Zoom out" aria-label="Zoom out" disabled>−</button>
      <select id="zoom" aria-label="Zoom" disabled>
        <option value="page-width">Fit width</option>
        <option value="page-fit">Fit page</option>
        <option value="0.5">50%</option><option value="0.75">75%</option>
        <option value="1">100%</option><option value="1.25">125%</option>
        <option value="1.5">150%</option><option value="2">200%</option>
        <option id="custom-zoom" value="custom" hidden></option>
      </select>
      <button id="zoom-in" title="Zoom in" aria-label="Zoom in" disabled>+</button>
    </div>
    <button id="reload" title="Reload PDF" aria-label="Reload PDF">↻</button>
  </header>
  <div id="viewer-container" role="main" aria-label="PDF document" title="Ctrl/Cmd+click to go to the TeX source" aria-busy="true" tabindex="0">
    <div id="viewer" class="pdfViewer"></div>
  </div>
  <div id="status" role="status" aria-live="polite">Loading PDF…</div>
  <script nonce="${nonce}">
  (() => {
    const api = acquireVsCodeApi();
    import(${JSON.stringify(resource('main.mjs'))}).then(({ start }) => start(api)).catch(error => {
      const message = 'Could not start PDF preview: ' + (error.message || String(error));
      const status = document.getElementById('status');
      status.textContent = message;
      status.classList.add('error');
      status.hidden = false;
      api.postMessage({ type: 'initializationError', message });
    });
  })();
  </script>
</body>
</html>`;
}
