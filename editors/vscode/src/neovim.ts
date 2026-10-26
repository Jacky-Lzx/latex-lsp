import * as fs from 'node:fs/promises';
import * as vscode from 'vscode';

const moduleName = 'texquill.linked_editing';
function literal(text: string): string {
  let delimiter = '=';
  while (text.includes(`]${delimiter}]`)) delimiter += '=';
  return `[${delimiter}[${text}]${delimiter}]`;
}

export class NeovimLinkedEditing implements vscode.Disposable {
  private script?: Promise<string>;
  private installed = false;
  private disposed = false;
  private readonly settings: vscode.Disposable;

  constructor(private readonly context: vscode.ExtensionContext, private readonly output: vscode.OutputChannel) {
    this.settings = vscode.Disposable.from(vscode.workspace.onDidChangeConfiguration(event => {
      if (this.installed && event.affectsConfiguration('editor.linkedEditing')) {
        void this.run(`local m=package.loaded['${moduleName}']; if m then m.clear() end`);
      }
    }), vscode.workspace.onDidCloseTextDocument(document => {
      if (this.installed && document.uri.scheme === 'file') {
        void this.run(`local m=package.loaded['${moduleName}']; if m then m.clear(${literal(document.uri.fsPath)}) end`);
      }
    }));
  }

  async arm(document: vscode.TextDocument, linked: vscode.LinkedEditingRanges | null | undefined): Promise<void> {
    if (this.disposed || !vscode.extensions.getExtension('asvetliakov.vscode-neovim')?.isActive
      || document.uri.scheme !== 'file' || linked?.ranges.length !== 2
      || !vscode.workspace.getConfiguration('editor', document).get<boolean>('linkedEditing')) return;
    const ranges = linked.ranges;
    const name = document.getText(ranges[0]);
    if (ranges.some(range => !range.isSingleLine || document.getText(range) !== name)) return;
    const data = { path: document.uri.fsPath, name, ranges: ranges.map(range => {
      const line = document.lineAt(range.start.line).text;
      return { row: range.start.line, first: Buffer.byteLength(line.slice(0, range.start.character)),
        last: Buffer.byteLength(line.slice(0, range.end.character)) };
    }) };
    this.script ??= fs.readFile(vscode.Uri.joinPath(this.context.extensionUri, 'dist/neovim-linked-editing.lua').fsPath, 'utf8');
    try {
      const script = await this.script;
      if (this.disposed) return;
      await this.run(`local key='${moduleName}'; if not package.loaded[key] then package.loaded[key]=(function()\n${script}\nend)() end; package.loaded[key].arm(vim.json.decode(${literal(JSON.stringify(data))}))`);
      this.installed = true;
    } catch (error) {
      this.output.appendLine(`Neovim linked editing: ${String(error)}`);
    }
  }

  private async run(code: string): Promise<void> {
    try { await vscode.commands.executeCommand('vscode-neovim.lua', code); }
    catch (error) { this.output.appendLine(`Neovim linked editing: ${String(error)}`); }
  }

  dispose(): void {
    this.disposed = true;
    this.settings.dispose();
    if (this.installed) void this.run(`local m=package.loaded['${moduleName}']; if m then m.dispose(); package.loaded['${moduleName}']=nil end`);
  }
}
