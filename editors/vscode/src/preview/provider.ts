import * as path from 'node:path';
import * as vscode from 'vscode';
import { previewHtml } from './html';

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
};

class PreviewSession implements vscode.Disposable {
  private ready = false;
  private dirty = true;
  private disposed = false;
  private readonly subscriptions: vscode.Disposable[] = [];
  readonly state: PreviewStatus;

  constructor(
    readonly pdf: vscode.Uri,
    readonly panel: vscode.WebviewPanel,
    assets: vscode.Uri,
    private readonly output: vscode.OutputChannel,
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
    }));
    panel.webview.html = previewHtml(panel.webview, assets);
  }

  refresh(): void {
    if (this.disposed) return;
    this.state.revision++;
    this.state.status = 'loading';
    this.state.renderedPages = [];
    delete this.state.error;
    this.dirty = true;
    void this.sendDocument();
  }

  private async sendDocument(): Promise<void> {
    if (this.disposed || !this.ready || !this.panel.visible || !this.dirty) return;
    this.dirty = false;
    const revision = this.state.revision;
    const resource = this.panel.webview.asWebviewUri(this.pdf);
    const url = resource.with({ query: `texquillRevision=${revision}` }).toString();
    try {
      const delivered = await this.panel.webview.postMessage({ type: 'document', url, revision, pdfUri: this.pdf.toString() });
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
  private readonly registration: vscode.Disposable;
  private readonly assets: vscode.Uri;

  constructor(context: vscode.ExtensionContext, private readonly output: vscode.OutputChannel) {
    this.assets = vscode.Uri.joinPath(context.extensionUri, 'dist/preview/assets');
    this.registration = vscode.window.registerWebviewPanelSerializer(pdfViewType, this);
  }

  private attach(pdf: vscode.Uri, panel: vscode.WebviewPanel): void {
    const key = pdf.toString();
    panel.title = path.basename(pdf.path);
    const session = new PreviewSession(pdf, panel, this.assets, this.output);
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
    this.attach(pdf, panel);
  }

  async open(pdf: vscode.Uri, preserveFocus = true): Promise<void> {
    await vscode.workspace.fs.stat(pdf);
    const existing = this.sessions.get(pdf.toString());
    if (existing) {
      existing.panel.reveal(existing.panel.viewColumn, preserveFocus);
      return;
    }
    const panel = vscode.window.createWebviewPanel(pdfViewType, path.basename(pdf.path), {
      viewColumn: vscode.ViewColumn.Beside,
      preserveFocus,
    }, { retainContextWhenHidden: true });
    this.attach(pdf, panel);
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
    for (const session of this.sessions.values()) {
      session.dispose();
      session.panel.dispose();
    }
    this.sessions.clear();
  }
}
