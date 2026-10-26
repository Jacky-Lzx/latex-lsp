import * as vscode from 'vscode';
import { PdfPreviewProvider } from './preview/provider';
import { type InverseSearch, type PdfLocation, isPdfPoint } from './preview/protocol';

type SearchResult<T> = { rootUri: string; pdfUri: string; locations: T[] };
type SourceLocation = { uri: string; range: { start: vscode.Position; end: vscode.Position } };
type Execute = <T>(command: string, args: unknown[]) => Promise<T>;

export class SyncTexNavigation {
  private forwardRequest = 0;
  private inverseRequest = 0;

  constructor(private readonly execute: Execute, private readonly previews: PdfPreviewProvider) {}

  async forward(uri?: vscode.Uri): Promise<SearchResult<PdfLocation>> {
    const active = vscode.window.activeTextEditor;
    const editor = !uri || active?.document.uri.toString() === uri.toString() ? active
      : vscode.window.visibleTextEditors.find(candidate => candidate.document.uri.toString() === uri.toString());
    if (!editor || editor.document.uri.scheme !== 'file' || !['latex', 'tex', 'plaintex'].includes(editor.document.languageId)) {
      throw new Error('Open and save a TeX source file before forward search.');
    }
    const request = ++this.forwardRequest;
    const result = await this.execute<SearchResult<PdfLocation>>('latex.forwardSearch', [{
      uri: editor.document.uri.toString(), position: editor.selection.active,
    }]);
    if (request !== this.forwardRequest) return result;
    const location = result.locations.find(isPdfPoint);
    if (!location) {
      void vscode.window.showInformationMessage('TeXquill: No PDF location found for this source line.');
      return result;
    }
    const pdf = vscode.Uri.parse(result.pdfUri, true);
    if (pdf.scheme !== 'file') throw new Error('SyncTeX did not return a local PDF.');
    await this.previews.navigate(pdf, location, { uri: result.rootUri, column: editor.viewColumn });
    return result;
  }

  async inverse(point: InverseSearch): Promise<SearchResult<SourceLocation> | undefined> {
    if (!point || typeof point.pdfUri !== 'string' || !isPdfPoint(point)) throw new Error('Invalid PDF location.');
    const pdf = vscode.Uri.parse(point.pdfUri, true);
    const source = this.previews.getSource(pdf);
    if (!source) throw new Error('Open this PDF from a TeX source with TeXquill: Open PDF first.');
    const current = () => this.previews.getStatus(pdf)?.revision === point.revision;
    if (!current()) return;
    const request = ++this.inverseRequest;
    const result = await this.execute<SearchResult<SourceLocation>>('latex.inverseSearch', [{
      uri: source.uri, page: point.page, x: point.x, y: point.y,
    }]);
    if (request !== this.inverseRequest || !current() || result.pdfUri !== point.pdfUri) return result;
    const location = result.locations[0];
    if (!location) {
      void vscode.window.showInformationMessage('TeXquill: No source location found at this PDF position.');
      return result;
    }
    const uri = vscode.Uri.parse(location.uri, true);
    if (uri.scheme !== 'file') throw new Error('SyncTeX did not return a local source file.');
    const document = await vscode.workspace.openTextDocument(uri);
    if (request !== this.inverseRequest || !current()) return result;
    const range = new vscode.Range(location.range.start.line, location.range.start.character,
      location.range.end.line, location.range.end.character);
    const visible = vscode.window.visibleTextEditors.find(editor => editor.document.uri.toString() === uri.toString());
    const column = visible?.viewColumn ?? this.previews.sourceColumn(pdf);
    const editor = await vscode.window.showTextDocument(document, { viewColumn: column, selection: range, preview: false });
    editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
    return result;
  }
}
