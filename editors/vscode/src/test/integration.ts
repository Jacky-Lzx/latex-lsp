import * as assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import * as vscode from 'vscode';

async function eventually<T>(description: string, operation: () => Thenable<T> | T, accepts: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 30_000;
  let last: unknown;
  while (Date.now() < deadline) {
    try {
      const result = await operation();
      if (accepts(result)) return result;
      last = result;
    } catch (error) { last = error; }
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw new Error(`Timed out: ${description}; last result: ${String(last)}`);
}

type Status = { status: string; buildId?: number; pdfUri?: string; stale?: boolean };

export async function run(): Promise<void> {
  const root = process.env.TEXQUILL_TEST_WORKSPACE!;
  const mainUri = vscode.Uri.file(path.join(root, 'main.tex'));
  const chapterUri = vscode.Uri.file(path.join(root, 'chapter.tex'));
  const document = await vscode.workspace.openTextDocument(mainUri);
  await vscode.window.showTextDocument(document);
  const extension = vscode.extensions.getExtension('Jacky-Lzx.texquill');
  assert.ok(extension, 'development extension is installed');
  await extension.activate();
  const status = () => vscode.commands.executeCommand<Status>('texquill.buildStatus', mainUri);
  assert.equal((await status())?.status, 'idle');
  console.log('PASS activation and command registration');

  const reference = document.positionAt(document.getText().indexOf('sec:intro') + 2);
  const definitions = await eventually('cross-file definition',
    () => vscode.commands.executeCommand<vscode.Location[]>('vscode.executeDefinitionProvider', mainUri, reference),
    result => !!result?.length);
  assert.equal(definitions![0].uri.toString(), chapterUri.toString());
  const rename = await vscode.commands.executeCommand<vscode.WorkspaceEdit>(
    'vscode.executeDocumentRenameProvider', mainUri, reference, 'sec:renamed');
  assert.ok(rename);
  assert.equal(rename.entries().length, 2, 'rename edits definition and reference');
  console.log('PASS cross-file definition and rename');

  const untitled = await vscode.workspace.openTextDocument({ language: 'latex', content: '\\sec' });
  await vscode.window.showTextDocument(untitled);
  const completions = await eventually('untitled completion',
    () => vscode.commands.executeCommand<vscode.CompletionList>('vscode.executeCompletionItemProvider', untitled.uri, new vscode.Position(0, 4)),
    result => !!result?.items.some(item => (typeof item.label === 'string' ? item.label : item.label.label).includes('section')));
  assert.ok(completions!.items.length > 0);
  console.log('PASS untitled document completion');

  const outline = await vscode.workspace.openTextDocument({
    language: 'latex',
    content: '\\section{Parent}\n\\subsection{}\n\\label{child}\n\\section{ \t }\n\\label{peer}',
  });
  const outlineEditor = await vscode.window.showTextDocument(outline);
  const outlineNames = (symbols: vscode.DocumentSymbol[]): string[] =>
    symbols.flatMap(symbol => [symbol.name, ...outlineNames(symbol.children)]);
  const outlineSymbols = await eventually('outline with empty headings',
    () => vscode.commands.executeCommand<vscode.DocumentSymbol[]>('vscode.executeDocumentSymbolProvider', outline.uri),
    result => !!result?.length);
  assert.deepEqual(outlineNames(outlineSymbols!), ['Parent', '\\subsection', 'child', '\\section', 'peer']);
  assert.equal(outlineSymbols![0].children[0].children[0].name, 'child');
  assert.equal(outlineSymbols![1].children[0].name, 'peer');
  const titlePosition = outline.positionAt(outline.getText().indexOf('\\subsection{') + '\\subsection{'.length);
  await outlineEditor.edit(edit => edit.insert(titlePosition, 'Recovered'));
  await eventually('outline refresh after filling an empty heading',
    () => vscode.commands.executeCommand<vscode.DocumentSymbol[]>('vscode.executeDocumentSymbolProvider', outline.uri),
    result => !!result && outlineNames(result).includes('Recovered'));
  console.log('PASS empty heading outlines and live title updates');

  const bib = await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(root, 'references.bib')));
  await vscode.window.showTextDocument(bib);
  const symbols = await eventually('BibTeX symbols',
    () => vscode.commands.executeCommand<vscode.DocumentSymbol[]>('vscode.executeDocumentSymbolProvider', bib.uri),
    result => !!result?.length);
  assert.ok(symbols!.some(symbol => symbol.name.includes('example')));
  console.log('PASS BibTeX language activation');

  await vscode.window.showTextDocument(document);
  const lenses = await eventually('build CodeLens',
    () => vscode.commands.executeCommand<vscode.CodeLens[]>('vscode.executeCodeLensProvider', mainUri),
    result => !!result?.some(lens => lens.command?.command === 'latex.build'));
  const build = lenses!.find(lens => lens.command?.command === 'latex.build')!.command!;
  await vscode.commands.executeCommand(build.command, ...(build.arguments ?? []));
  const first = await eventually('CodeLens build completes', status, result => result?.status === 'success');
  assert.ok(first!.pdfUri);
  assert.ok((await fs.stat(vscode.Uri.parse(first!.pdfUri!).fsPath)).size > 0);
  console.log('PASS CodeLens executes server command and real pdfLaTeX build');
  await vscode.commands.executeCommand('texquill.openPdf', mainUri);
  assert.ok(vscode.window.tabGroups.all.flatMap(group => group.tabs).some(tab => {
    const input = tab.input;
    return (input instanceof vscode.TabInputText || input instanceof vscode.TabInputCustom)
      && input.uri.toString() === first!.pdfUri;
  }), 'Open PDF opens the compiled artifact in VS Code');
  console.log('PASS Open PDF command');

  await vscode.commands.executeCommand('texquill.rebuild', mainUri);
  const second = await eventually('rebuild completes', status,
    result => result?.status === 'success' && result.buildId !== first!.buildId);
  assert.ok(second);
  console.log('PASS rebuild command');

  const config = vscode.workspace.getConfiguration('texquill');
  await config.update('settings', {
    distribution: { enabled: false },
    build: { executable: 'pdflatex', outputDirectory: 'build', onSave: true },
  }, vscode.ConfigurationTarget.Global);
  // The configuration event is asynchronous; a later notification round trip
  // and observable build result establish that live settings reached the server.
  await new Promise(resolve => setTimeout(resolve, 500));
  const editor = await vscode.window.showTextDocument(document);
  await editor.edit(edit => edit.insert(new vscode.Position(3, 0), 'Updated. '));
  await document.save();
  await eventually('save triggers build after live configuration', status,
    result => result?.status === 'success' && result.buildId !== second!.buildId);
  console.log('PASS live configuration and save-triggered compilation');

  await config.update('settings', {
    distribution: { enabled: false },
    build: { executable: process.env.TEXQUILL_TEST_NODE, arguments: [path.join(root, 'slow.cjs')], outputDirectory: 'build' },
  }, vscode.ConfigurationTarget.Global);
  await new Promise(resolve => setTimeout(resolve, 500));
  await vscode.commands.executeCommand('texquill.build', mainUri);
  const compilerPid = await eventually('compiler subprocess starts',
    async () => Number(await fs.readFile(path.join(root, 'slow.pid'), 'utf8')), pid => pid > 0);
  await vscode.commands.executeCommand('texquill.cancelBuild', mainUri);
  await eventually('build cancellation', status, result => result?.status === 'cancelled');
  await eventually('compiler subprocess exits', () => {
    try { process.kill(compilerPid, 0); return false; }
    catch (error) { return (error as NodeJS.ErrnoException).code === 'ESRCH'; }
  }, exited => exited);
  console.log('PASS cancellation terminates the compiler subprocess');

  await editor.edit(edit => edit.insert(new vscode.Position(3, 0), '\\ref{missing-label} '));
  await eventually('pull diagnostics update after editing',
    () => vscode.languages.getDiagnostics(mainUri),
    diagnostics => diagnostics.some(diagnostic => diagnostic.message.includes('missing-label')));
  await vscode.commands.executeCommand('texquill.restart');
  await eventually('unsaved document survives restart',
    () => vscode.languages.getDiagnostics(mainUri),
    diagnostics => diagnostics.some(diagnostic => diagnostic.message.includes('missing-label')));
  const afterRestart = await vscode.commands.executeCommand<vscode.Location[]>(
    'vscode.executeDefinitionProvider', mainUri,
    document.positionAt(document.getText().indexOf('sec:intro') + 2));
  assert.ok(afterRestart?.length);
  console.log('PASS diagnostics, restart, and resynchronization of unsaved documents');

  const serverPath = config.get<string>('serverPath')!;
  await config.update('serverPath', path.join(root, 'missing-server'), vscode.ConfigurationTarget.Global);
  await assert.rejects(() => Promise.resolve(vscode.commands.executeCommand('texquill.restart')),
    /Check texquill.serverPath and PATH/);
  await config.update('serverPath', serverPath, vscode.ConfigurationTarget.Global);
  await vscode.commands.executeCommand('texquill.restart');
  const recovered = await eventually('server recovers after invalid executable',
    () => vscode.commands.executeCommand<vscode.Location[]>('vscode.executeDefinitionProvider', mainUri,
      document.positionAt(document.getText().indexOf('sec:intro') + 2)), result => !!result?.length);
  assert.ok(recovered);
  console.log('PASS startup failure recovery without reloading VS Code');
  console.log('All VS Code integration checks passed.');
}
