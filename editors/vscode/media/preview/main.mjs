import { AnnotationMode, GlobalWorkerOptions, getDocument } from './vendor/pdf.mjs';
import { EventBus, PDFLinkService, PDFViewer } from './vendor/pdf_viewer.mjs';

export async function start(vscode) {
  const config = JSON.parse(document.querySelector('meta[name="texquill-pdf-config"]').content);
  // VS Code webviews require a self-contained blob worker; PDF.js's default
  // cross-origin wrapper uses an import that webview workers cannot perform.
  const workerResponse = await fetch(config.worker);
  if (!workerResponse.ok) throw new Error(`PDF worker could not be loaded (${workerResponse.status}).`);
  const workerUrl = URL.createObjectURL(new Blob([await workerResponse.text()], { type: 'text/javascript' }));
  GlobalWorkerOptions.workerSrc = workerUrl;
  window.addEventListener('pagehide', () => URL.revokeObjectURL(workerUrl), { once: true });
  const element = id => document.getElementById(id);
  const container = element('viewer-container');
  const status = element('status');
  const eventBus = new EventBus();
  const links = new PDFLinkService({ eventBus, ignoreDestinationZoom: true });
  links.externalLinkEnabled = false;
  const viewer = new PDFViewer({
    container, viewer: element('viewer'), eventBus, linkService: links,
    annotationMode: AnnotationMode.ENABLE, maxCanvasPixels: 16 * 1024 * 1024,
  });
  links.setViewer(viewer);
  let revision = 0;
  let documentRevision = 0;
  let documentState = 'loading';
  let loadingTask;
  let pdf;
  let restoring = false;
  const persisted = vscode.getState();
  let currentPdfUri = persisted?.pdfUri;
  let saved = normalizeState(persisted);
  let resizeFrame;

  function normalizeState(state) {
    const scale = state?.scale;
    return {
      page: Number.isSafeInteger(state?.page) && state.page > 0 ? state.page : 1,
      scale: ['page-width', 'page-fit'].includes(scale) || Number(scale) >= .25 && Number(scale) <= 4 ? String(scale) : 'page-width',
      left: Number.isFinite(state?.left) ? state.left : 0,
      top: Number.isFinite(state?.top) ? state.top : null,
    };
  }

  function report(type, data = {}) {
    vscode.postMessage({ type, revision: documentRevision, ...data });
  }

  function busy(message, error = false) {
    status.textContent = message;
    status.classList.toggle('error', error);
    status.hidden = !message;
    container.setAttribute('aria-busy', String(!error && !!message));
  }

  function controls() {
    const count = pdf?.numPages ?? 0;
    element('page').disabled = !count;
    element('page').max = String(count);
    element('page').value = String(viewer.currentPageNumber || 1);
    element('page-count').textContent = count ? `/ ${count}` : '/ —';
    element('previous').disabled = !count || viewer.currentPageNumber <= 1;
    element('next').disabled = !count || viewer.currentPageNumber >= count;
    for (const id of ['zoom', 'zoom-in', 'zoom-out']) element(id).disabled = !count;
    const scale = String(viewer.currentScaleValue);
    const standard = [...element('zoom').options].find(option => option.value === scale);
    const custom = element('custom-zoom');
    custom.hidden = !!standard;
    custom.textContent = `${Math.round(viewer.currentScale * 100)}%`;
    element('zoom').value = standard ? scale : 'custom';
  }

  function restoreView() {
    viewer.currentScaleValue = saved.scale;
    const number = Math.min(saved.page, pdf.numPages);
    viewer.scrollPageIntoView({ pageNumber: number });
    const page = viewer.getPageView(number - 1).div;
    if (saved.top !== null) container.scrollTop = page.offsetTop + saved.top * page.offsetHeight;
    container.scrollLeft = page.offsetLeft + saved.left * page.offsetWidth;
    viewer.update();
  }

  eventBus.on('pagesinit', () => {
    restoreView();
    controls();
  });
  eventBus.on('pagesloaded', () => {
    // Mixed page sizes may become known only after pagesinit.
    if (restoring && pdf) restoreView();
    restoring = false;
  });
  eventBus.on('pagerendered', event => {
    if (documentState !== 'ready' || !pdf || viewer.getPageView(event.pageNumber - 1) !== event.source) return;
    if (event.error) {
      busy(`Could not render this page: ${event.error.message ?? event.error}`, true);
      report('error', { message: status.textContent });
      return;
    }
    busy('');
    report('rendered', { page: event.pageNumber });
  });
  eventBus.on('pagechanging', controls);
  eventBus.on('scalechanging', controls);
  eventBus.on('updateviewarea', ({ location }) => {
    if (!pdf || restoring) return;
    const page = viewer.getPageView(location.pageNumber - 1).div;
    saved = {
      page: location.pageNumber, scale: String(viewer.currentScaleValue),
      left: (container.scrollLeft - page.offsetLeft) / page.offsetWidth,
      top: (container.scrollTop - page.offsetTop) / page.offsetHeight,
    };
    vscode.setState({ ...saved, pdfUri: currentPdfUri });
    report('view', { page: saved.page, scale: saved.scale });
  });

  async function loadDocument(message) {
    const current = ++revision;
    currentPdfUri = message.pdfUri ?? currentPdfUri;
    vscode.setState({ ...saved, pdfUri: currentPdfUri });
    const pending = loadingTask;
    loadingTask = undefined;
    if (pending && pending !== pdf?.loadingTask) void pending.destroy().catch(() => {});
    documentState = 'loading';
    busy('Loading PDF…');
    try {
      const task = loadingTask = getDocument({
        url: message.url,
        cMapUrl: `${config.resources}cmaps/`, cMapPacked: true,
        standardFontDataUrl: `${config.resources}standard_fonts/`,
        wasmUrl: `${config.resources}wasm/`, iccUrl: `${config.resources}iccs/`,
        isEvalSupported: false, enableXfa: false,
      });
      const next = await task.promise;
      if (current !== revision) { await next.loadingTask.destroy(); return; }
      const previous = pdf;
      documentRevision = message.revision;
      restoring = true;
      pdf = next;
      documentState = 'ready';
      viewer.setDocument(next);
      links.setDocument(next);
      report('loaded', { pages: next.numPages });
      if (previous) void previous.loadingTask.destroy().catch(() => {});
    } catch (error) {
      if (current !== revision) return;
      documentRevision = message.revision;
      documentState = 'error';
      busy(`Could not load PDF: ${error.message ?? error}`, true);
      report('error', { message: status.textContent });
    }
  }

  function page(number) {
    if (!pdf) return;
    restoring = false;
    viewer.currentPageNumber = Math.max(1, Math.min(pdf.numPages, Math.trunc(number) || 1));
  }
  function zoom(factor) {
    if (!pdf) return;
    restoring = false;
    viewer.currentScaleValue = String(Math.max(.25, Math.min(4, viewer.currentScale * factor)));
  }
  element('previous').addEventListener('click', () => page(viewer.currentPageNumber - 1));
  element('next').addEventListener('click', () => page(viewer.currentPageNumber + 1));
  element('page').addEventListener('change', event => page(Number(event.target.value)));
  element('page').addEventListener('keydown', event => { if (event.key === 'Enter') page(Number(event.target.value)); });
  element('zoom').addEventListener('change', event => {
    if (pdf && event.target.value !== 'custom') {
      restoring = false;
      viewer.currentScaleValue = event.target.value;
    }
  });
  element('zoom-in').addEventListener('click', () => zoom(1.2));
  element('zoom-out').addEventListener('click', () => zoom(1 / 1.2));
  element('reload').addEventListener('click', () => vscode.postMessage({ type: 'reload' }));
  for (const event of ['wheel', 'pointerdown', 'keydown']) {
    container.addEventListener(event, () => { restoring = false; }, { passive: true });
  }
  new ResizeObserver(() => {
    cancelAnimationFrame(resizeFrame);
    resizeFrame = requestAnimationFrame(() => {
      if (pdf && ['page-width', 'page-fit'].includes(viewer.currentScaleValue)) {
        viewer.currentScaleValue = viewer.currentScaleValue;
        viewer.update();
      }
    });
  }).observe(container);
  window.addEventListener('message', ({ data }) => {
    if (data?.type === 'document' && typeof data.url === 'string' && Number.isSafeInteger(data.revision)) {
      void loadDocument(data);
    }
  });
  // Loading and refreshing only update the document. They never focus or reveal
  // the webview; keyboard focus changes only when the user interacts with it.
  vscode.postMessage({ type: 'ready' });
}
