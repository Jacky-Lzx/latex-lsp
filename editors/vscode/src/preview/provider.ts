import * as path from 'node:path';
import * as vscode from 'vscode';
import { previewHtml } from './html';
import { type InverseSearch, type PdfLocation, type PreviewSource, isPdfPoint } from './protocol';

export const pdfViewType = 'texquill.pdfPreview';

export type PreviewStatus = {
  pdfUri: string;
  revision: number;
  status: 'loading' | 'ready' | 'error';
  pages: number;
  renderedPages: number[];
  page: number;
  scale: string;
  error?: string;
  navigation?: PdfLocation;
};

class PreviewSession implements vscode.Disposable {
  private ready = false;
  private dirty = true;
  private disposed = false;
  private navigationId = 0;
  private pendingNavigation?: { id: number; location: PdfLocation };
  private readonly subscriptions: vscode.Disposable[] = [];
  readonly state: PreviewStatus;

  constructor(
    readonly pdf: vscode.Uri,
    readonly panel: vscode.WebviewPanel,
    assets: vscode.Uri,
    private readonly output: vscode.OutputChannel,
    public source: PreviewSource | undefined,
    inverseSearch: (point: InverseSearch) => void,
  ) {
    this.state = { pdfUri: pdf.toString(), revision: 1, status: 'loading', pages: 0, renderedPages: [], page: 1, scale: 'page-width' };
    panel.webview.options = {
      enableScripts: true,
      localResourceRoots: [assets, vscode.Uri.joinPath(pdf, '..')],
    };
    this.subscriptions.push(panel.webview.onDidReceiveMessage(message => {
      if (!message || typeof message !== 'object') return;
      if (message.type === 'ready') {
        this.ready = true;
        void this.sendDocument();
      } else if (message.type === 'reload') {
        this.refresh();
      } else if (message.type === 'initializationError' && typeof message.message === 'string') {
        this.state.status = 'error';
        this.state.error = message.message;
        output.appendLine(message.message);
      } else if (message.revision === this.state.revision) {
        if (message.type === 'loaded' && Number.isSafeInteger(message.pages) && message.pages > 0) {
          this.state.pages = message.pages;
        } else if (message.type === 'rendered' && Number.isSafeInteger(message.page) && message.page > 0) {
          this.state.status = 'ready';
          if (!this.state.renderedPages.includes(message.page)) this.state.renderedPages.push(message.page);
          void this.sendNavigation();
        } else if (message.type === 'navigated' && message.id === this.navigationId && isPdfPoint(message.location)) {
          this.state.navigation = message.location;
        } else if (message.type === 'inverseSearch' && this.state.status === 'ready' && isPdfPoint(message)) {
          inverseSearch({ pdfUri: pdf.toString(), revision: this.state.revision, page: message.page, x: message.x, y: message.y });
        } else if (message.type === 'view' && Number.isSafeInteger(message.page) && typeof message.scale === 'string') {
          this.state.page = message.page;
          this.state.scale = message.scale;
        } else if (message.type === 'error' && typeof message.message === 'string') {
          this.state.status = 'error';
          this.state.error = message.message;
          output.appendLine(`PDF preview: ${message.message}`);
        }
      }
    }), panel.onDidChangeViewState(() => {
      if (panel.visible && this.dirty) void this.sendDocument();
      if (panel.visible) void this.sendNavigation();
    }));
    panel.webview.html = previewHtml(panel.webview, assets);
  }

  refresh(): void {
    if (this.disposed) return;
    this.state.revision++;
    this.state.status = 'loading';
    this.state.renderedPages = [];
    delete this.state.error;
    delete this.state.navigation;
    this.pendingNavigation = undefined;
    this.navigationId++;
    this.dirty = true;
    void this.sendDocument();
  }

  navigate(location: PdfLocation): void {
    this.pendingNavigation = { id: ++this.navigationId, location };
    void this.sendNavigation();
  }

  private async sendNavigation(): Promise<void> {
    if (this.disposed || !this.panel.visible || this.state.status !== 'ready' || !this.pendingNavigation) return;
    const navigation = this.pendingNavigation;
    this.pendingNavigation = undefined;
    try {
      const delivered = await this.panel.webview.postMessage({ type: 'navigate', revision: this.state.revision, ...navigation });
      if (!delivered && navigation.id === this.navigationId) this.pendingNavigation = navigation;
    } catch (error) {
      if (!this.disposed) this.output.appendLine(`PDF navigation: ${String(error)}`);
    }
  }

  private async sendDocument(): Promise<void> {
    if (this.disposed || !this.ready || !this.panel.visible || !this.dirty) return;
    this.dirty = false;
    const revision = this.state.revision;
    const resource = this.panel.webview.asWebviewUri(this.pdf);
    const url = resource.with({ query: `texquillRevision=${revision}` }).toString();
    try {
      const delivered = await this.panel.webview.postMessage({ type: 'document', url, revision, pdfUri: this.pdf.toString(), source: this.source });
      if (!delivered && revision === this.state.revision) this.dirty = true;
    } catch (error) {
      if (!this.disposed) this.output.appendLine(`PDF preview: ${String(error)}`);
    }
  }

  dispose(): void {
    this.disposed = true;
    for (const subscription of this.subscriptions) subscription.dispose();
  }
}

export class PdfPreviewProvider implements vscode.WebviewPanelSerializer, vscode.Disposable {
  private readonly sessions = new Map<string, PreviewSession>();
  private navigationRequest = 0;
  private readonly registration: vscode.Disposable;
  private readonly assets: vscode.Uri;
  private readonly inverseSearch = new vscode.EventEmitter<InverseSearch>();
  readonly onDidRequestInverseSearch = this.inverseSearch.event;

  constructor(context: vscode.ExtensionContext, private readonly output: vscode.OutputChannel) {
    this.assets = vscode.Uri.joinPath(context.extensionUri, 'dist/preview/assets');
    this.registration = vscode.window.registerWebviewPanelSerializer(pdfViewType, this);
  }

  private attach(pdf: vscode.Uri, panel: vscode.WebviewPanel, source?: PreviewSource): void {
    const key = pdf.toString();
    panel.title = path.basename(pdf.path);
    const session = new PreviewSession(pdf, panel, this.assets, this.output, source, point => this.inverseSearch.fire(point));
    this.sessions.set(key, session);
    panel.onDidDispose(() => {
      session.dispose();
      if (this.sessions.get(key) === session) this.sessions.delete(key);
    });
  }

  async deserializeWebviewPanel(panel: vscode.WebviewPanel, state: unknown): Promise<void> {
    if (!state || typeof state !== 'object' || !('pdfUri' in state) || typeof state.pdfUri !== 'string') {
      panel.dispose();
      return;
    }
    const pdf = vscode.Uri.parse(state.pdfUri, true);
    if (pdf.scheme !== 'file') {
      panel.dispose();
      return;
    }
    const source = 'source' in state ? state.source as PreviewSource | undefined : undefined;
    this.attach(pdf, panel, typeof source?.uri === 'string' && vscode.Uri.parse(source.uri).scheme === 'file' ? source : undefined);
  }

  async open(pdf: vscode.Uri, preserveFocus = true, source?: PreviewSource): Promise<void> {
    await vscode.workspace.fs.stat(pdf);
    const existing = this.sessions.get(pdf.toString());
    if (existing) {
      if (source) {
        existing.source = source;
        void existing.panel.webview.postMessage({ type: 'source', source });
      }
      const column = preserveFocus && vscode.window.activeTextEditor?.viewColumn === existing.panel.viewColumn
        ? vscode.ViewColumn.Beside : existing.panel.viewColumn;
      existing.panel.reveal(column, preserveFocus);
      return;
    }
    const panel = vscode.window.createWebviewPanel(pdfViewType, path.basename(pdf.path), {
      viewColumn: vscode.ViewColumn.Beside,
      preserveFocus,
    }, { retainContextWhenHidden: true });
    this.attach(pdf, panel, source);
  }

  async navigate(pdf: vscode.Uri, location: PdfLocation, source: PreviewSource): Promise<void> {
    const request = ++this.navigationRequest;
    await this.open(pdf, true, source);
    if (request === this.navigationRequest) this.sessions.get(pdf.toString())?.navigate(location);
  }

  getSource(pdf: vscode.Uri): PreviewSource | undefined {
    const source = this.sessions.get(pdf.toString())?.source;
    return source ? { ...source } : undefined;
  }

  sourceColumn(pdf: vscode.Uri): vscode.ViewColumn {
    const session = this.sessions.get(pdf.toString());
    const column = session?.source?.column;
    const groups = vscode.window.tabGroups.all.filter(group => group.viewColumn !== session?.panel.viewColumn);
    return groups.find(group => group.viewColumn === column)?.viewColumn ?? groups[0]?.viewColumn ?? vscode.ViewColumn.Beside;
  }

  refresh(pdf: vscode.Uri): void {
    this.sessions.get(pdf.toString())?.refresh();
  }

  getStatus(pdf: vscode.Uri): PreviewStatus | undefined {
    const state = this.sessions.get(pdf.toString())?.state;
    return state ? { ...state, renderedPages: [...state.renderedPages] } : undefined;
  }

  dispose(): void {
    this.registration.dispose();
    this.inverseSearch.dispose();
    for (const session of this.sessions.values()) {
      session.dispose();
      session.panel.dispose();
    }
    this.sessions.clear();
  }
}
