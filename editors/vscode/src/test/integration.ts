import * as assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { pdfViewType, type PdfPreviewProvider } from '../preview/provider';
import { type PdfLocation } from '../preview/protocol';

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
  throw new Error(`Timed out: ${description}; last result: ${JSON.stringify(last)}`);
}

type Status = { status: string; buildId?: number; pdfUri?: string; stale?: boolean };

export async function run(): Promise<void> {
  const root = process.env.TEXQUILL_TEST_WORKSPACE!;
  const mainUri = vscode.Uri.file(path.join(root, 'main.tex'));
  const chapterUri = vscode.Uri.file(path.join(root, 'chapter.tex'));
  const document = await vscode.workspace.openTextDocument(mainUri);
  await vscode.window.showTextDocument(document);
  const extension = vscode.extensions.getExtension<{ previews: PdfPreviewProvider }>('Jacky-Lzx.texquill');
  assert.ok(extension, 'development extension is installed');
  const { previews } = await extension.activate();
  const status = () => vscode.commands.executeCommand<Status>('texquill.buildStatus', mainUri);
  assert.equal((await status())?.status, 'idle');
  console.log('PASS activation and command registration');

  const linked = await vscode.workspace.openTextDocument({ language: 'latex',
    content: '\\begin{quote}\n\\begin{quote}Nested\\end{quote}\n\\end{quote}\n' });
  const linkedEditor = await vscode.window.showTextDocument(linked);
  const linkedConfig = vscode.workspace.getConfiguration('editor', linked);
  assert.equal(linkedConfig.get('linkedEditing'), true, 'TeX linked editing is enabled by default');
  linkedEditor.selection = new vscode.Selection(0, 9, 0, 9);
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('type', { text: 'x' });
  await eventually('typing in begin updates only its matching end', () => linked.getText(),
    text => text === '\\begin{quxote}\n\\begin{quote}Nested\\end{quote}\n\\end{quxote}\n');
  await vscode.commands.executeCommand('undo');
  await eventually('undo restores both linked names', () => linked.getText(),
    text => text === '\\begin{quote}\n\\begin{quote}Nested\\end{quote}\n\\end{quote}\n');
  await vscode.window.showTextDocument(linked, { selection: new vscode.Range(2, 10, 2, 10) });
  await new Promise(resolve => setTimeout(resolve, 800));
  assert.deepEqual(vscode.window.activeTextEditor?.selection.active, new vscode.Position(2, 10));
  await vscode.commands.executeCommand('type', { text: '*' });
  await eventually('typing in end updates only its matching begin', () => linked.getText(),
    text => text === '\\begin{quote*}\n\\begin{quote}Nested\\end{quote}\n\\end{quote*}\n');
  await vscode.window.showTextDocument(linked, { selection: new vscode.Range(0, 7, 0, 13) });
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('type', { text: 'itemize' });
  await eventually('replacing the whole name updates both delimiters', () => linked.getText(),
    text => text === '\\begin{itemize}\n\\begin{quote}Nested\\end{quote}\n\\end{itemize}\n');
  console.log('PASS automatic begin/end linked typing, whole-name replacement, nested isolation, and undo');
  await linkedConfig.update('linkedEditing', false, vscode.ConfigurationTarget.Global, true);
  await eventually('explicit user opt-out disables linked editing',
    () => vscode.workspace.getConfiguration('editor', linked).get('linkedEditing'), enabled => enabled === false);
  await vscode.window.showTextDocument(linked, { selection: new vscode.Range(0, 9, 0, 9) });
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('type', { text: 'z' });
  assert.equal(linked.getText(), '\\begin{itzemize}\n\\begin{quote}Nested\\end{quote}\n\\end{itemize}\n');
  await linkedConfig.update('linkedEditing', undefined, vscode.ConfigurationTarget.Global, true);
  for (const languageId of ['latex', 'tex', 'plaintex']) {
    assert.equal(vscode.workspace.getConfiguration('editor', { uri: mainUri, languageId }).get('linkedEditing'), true);
  }
  assert.equal(vscode.workspace.getConfiguration('editor', { uri: mainUri, languageId: 'bibtex' }).get('linkedEditing'), false);
  console.log('PASS linked editing respects explicit user opt-out and stays scoped to TeX languages');

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

  let sourceEditor = await vscode.window.showTextDocument(document);
  const sourceGroup = vscode.window.tabGroups.activeTabGroup;
  const sourceSelection = new vscode.Selection(reference, reference);
  sourceEditor.selection = sourceSelection;
  const assertSourceFocus = (selection: vscode.Selection) => {
    assert.equal(vscode.window.activeTextEditor?.document.uri.toString(), mainUri.toString(),
      'focus remains in the TeX editor');
    assert.equal(vscode.window.tabGroups.activeTabGroup, sourceGroup);
    assert.deepEqual(vscode.window.activeTextEditor?.selection, selection, 'TeX cursor is preserved');
  };
  const lenses = await eventually('build CodeLens',
    () => vscode.commands.executeCommand<vscode.CodeLens[]>('vscode.executeCodeLensProvider', mainUri),
    result => !!result?.some(lens => lens.command?.command === 'latex.build'));
  const build = lenses!.find(lens => lens.command?.command === 'latex.build')!.command!;
  await vscode.commands.executeCommand(build.command, ...(build.arguments ?? []));
  const first = await eventually('CodeLens build completes', status, result => result?.status === 'success');
  assert.ok(first!.pdfUri);
  assert.ok((await fs.stat(vscode.Uri.parse(first!.pdfUri!).fsPath)).size > 0);
  assertSourceFocus(sourceSelection);
  console.log('PASS CodeLens executes server command and real pdfLaTeX build');
  const pdfUri = vscode.Uri.parse(first!.pdfUri!);
  await vscode.commands.executeCommand('texquill.openPdf', mainUri);
  const preview = () => previews.getStatus(pdfUri);
  const loaded = await eventually('built-in PDF viewer renders actual pages', preview,
    result => result?.status === 'ready' && result.pages === 2 && result.renderedPages.includes(1));
  const isPdf = (tab: vscode.Tab) => tab.input instanceof vscode.TabInputWebview
    && tab.input.viewType.endsWith(pdfViewType);
  const pdfGroup = vscode.window.tabGroups.all.find(group => group.tabs.some(isPdf));
  assert.ok(pdfGroup, `Open PDF uses the built-in webview: ${JSON.stringify(vscode.window.tabGroups.all.flatMap(group => group.tabs).map(tab => ({ label: tab.label, input: tab.input })))}`);
  assert.ok(pdfGroup.viewColumn > sourceGroup.viewColumn, 'PDF opens beside the TeX editor');
  assert.equal(pdfGroup.tabs.find(isPdf)!.isPreview, false, 'PDF stays open as a pinned tab');
  await new Promise(resolve => setTimeout(resolve, 200));
  assertSourceFocus(sourceSelection);
  const groupCount = vscode.window.tabGroups.all.length;
  await vscode.commands.executeCommand('texquill.openPdf');
  assert.equal(vscode.window.tabGroups.all.length, groupCount, 'reopening PDF reuses the side group');
  assert.equal(vscode.window.tabGroups.all.flatMap(group => group.tabs).filter(isPdf).length, 1);
  assertSourceFocus(sourceSelection);
  console.log('PASS built-in PDF rendering beside TeX without moving focus or duplicating tabs');

  type Forward = { rootUri: string; pdfUri: string; locations: PdfLocation[] };
  const pageTwoSelection = new vscode.Selection(5, 3, 5, 3);
  sourceEditor.selection = pageTwoSelection;
  const forward = await vscode.commands.executeCommand<Forward>('texquill.forwardSearch');
  assert.equal(forward!.pdfUri, pdfUri.toString());
  assert.equal(forward!.locations[0].page, 2);
  await eventually('forward search scrolls and highlights page two', preview,
    result => result?.page === 2 && result.navigation?.page === 2 && result.renderedPages.includes(2));
  assertSourceFocus(pageTwoSelection);
  console.log('PASS real SyncTeX forward search highlights page two without moving TeX focus');

  const chapter = await vscode.workspace.openTextDocument(chapterUri);
  const chapterEditor = await vscode.window.showTextDocument(chapter, { viewColumn: sourceGroup.viewColumn });
  chapterEditor.selection = new vscode.Selection(0, 6, 0, 6);
  const chapterForward = await vscode.commands.executeCommand<Forward>('texquill.forwardSearch', chapterUri);
  assert.equal(chapterForward!.rootUri, mainUri.toString());
  await eventually('included source forwards to its PDF', preview,
    result => result?.navigation?.page === 1 && result.page === 1);
  assert.equal(vscode.window.activeTextEditor, chapterEditor);
  await previews.open(pdfUri, false);
  const hit = chapterForward!.locations[0];
  const inverse = await vscode.commands.executeCommand<{ locations: vscode.Location[] }>('texquill.inverseSearch', {
    pdfUri: pdfUri.toString(), revision: preview()!.revision, page: hit.page, x: hit.x, y: hit.y,
  });
  assert.equal(inverse!.locations[0].uri, chapterUri.toString());
  assert.equal(vscode.window.activeTextEditor?.document.uri.toString(), chapterUri.toString());
  assert.equal(vscode.window.activeTextEditor?.selection.active.line, 0);
  assert.equal(vscode.window.tabGroups.activeTabGroup, sourceGroup, 'inverse search reuses the source group');
  assert.ok(pdfGroup.tabs.some(isPdf), 'inverse search preserves the side PDF');
  assert.equal((await vscode.commands.executeCommand('texquill.inverseSearch', {
    pdfUri: pdfUri.toString(), revision: preview()!.revision - 1, ...hit,
  })), undefined, 'old PDF revisions cannot initiate an inverse jump');
  console.log('PASS real SyncTeX inverse search returns to an included file in the source group');
  sourceEditor = await vscode.window.showTextDocument(document, { viewColumn: sourceGroup.viewColumn });
  sourceEditor.selection = sourceSelection;
  await sourceEditor.edit(edit => edit.insert(new vscode.Position(3, 0), 'Unsaved. '));
  await assert.rejects(() => Promise.resolve(vscode.commands.executeCommand('texquill.forwardSearch')), /unsaved|save|changed/i);
  await assert.rejects(() => Promise.resolve(vscode.commands.executeCommand('texquill.inverseSearch', {
    pdfUri: pdfUri.toString(), revision: preview()!.revision, ...hit,
  })), /unsaved|save|changed/i);
  await sourceEditor.edit(edit => edit.delete(new vscode.Range(3, 0, 3, 'Unsaved. '.length)));
  await document.save();
  sourceEditor.selection = sourceSelection;
  assertSourceFocus(sourceSelection);
  console.log('PASS SyncTeX rejects unsaved sources in both directions');

  await vscode.commands.executeCommand('texquill.rebuild', mainUri);
  const second = await eventually('rebuild completes', status,
    result => result?.status === 'success' && result.buildId !== first!.buildId);
  assert.ok(second);
  const rebuilt = await eventually('PDF refreshes after rebuild', preview,
    result => result?.status === 'ready' && result.revision > loaded!.revision && result.renderedPages.length > 0);
  assertSourceFocus(sourceSelection);
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
  const savedSelection = editor.selection;
  await document.save();
  await eventually('save triggers build after live configuration', status,
    result => result?.status === 'success' && result.buildId !== second!.buildId);
  await eventually('PDF refreshes after save-triggered compilation', preview,
    result => result?.status === 'ready' && result.revision > rebuilt!.revision && result.renderedPages.length > 0);
  assertSourceFocus(savedSelection);
  assert.ok(pdfGroup.tabs.some(isPdf), 'PDF remains open after build on save');
  console.log('PASS live configuration and save-triggered compilation without moving TeX focus');

  const previous = await status();
  await previews.open(pdfUri, false);
  await vscode.commands.executeCommand('texquill.rebuild', mainUri);
  const previousRevision = preview()!.revision;
  await eventually('rebuild while reading the PDF', status,
    result => result?.status === 'success' && result.buildId !== previous!.buildId);
  await eventually('PDF refresh while its editor is focused', preview,
    result => result?.status === 'ready' && result.revision > previousRevision);
  assert.equal(vscode.window.tabGroups.activeTabGroup, pdfGroup,
    'a PDF that was already focused stays focused through a rebuild');
  await vscode.window.showTextDocument(document, { viewColumn: sourceGroup.viewColumn });
  console.log('PASS an intentionally focused PDF stays focused on rebuild');

  const revision = preview()!.revision;
  for (let index = 0; index < 3; index++) previews.refresh(pdfUri);
  await eventually('only the newest rapid PDF reload is displayed', preview,
    result => result?.status === 'ready' && result.revision === revision + 3 && result.pages === 2);
  assertSourceFocus(savedSelection);
  console.log('PASS consecutive PDF reloads settle on the newest document');

  await vscode.window.showTextDocument(bib, { viewColumn: pdfGroup.viewColumn, preserveFocus: true, preview: false });
  await eventually('PDF is hidden behind another tab', () => pdfGroup.activeTab, tab => !!tab && !isPdf(tab));
  const hiddenRevision = preview()!.revision;
  previews.refresh(pdfUri);
  assert.equal(preview()!.revision, hiddenRevision + 1);
  assert.equal(pdfGroup.activeTab!.label, path.basename(bib.uri.path), 'refresh does not reveal a hidden PDF');
  assertSourceFocus(savedSelection);
  await previews.open(pdfUri);
  await eventually('hidden refresh is delivered when the PDF is shown', preview,
    result => result?.status === 'ready' && result.revision === hiddenRevision + 1);
  assertSourceFocus(savedSelection);
  console.log('PASS hidden PDF refresh is deferred without changing tabs or focus');

  const validPdf = await fs.readFile(pdfUri.fsPath);
  try {
    await fs.writeFile(pdfUri.fsPath, 'This is not a PDF.');
    previews.refresh(pdfUri);
    await eventually('invalid PDF reports a recoverable error', preview,
      result => result?.status === 'error' && !!result.error);
    assertSourceFocus(savedSelection);
  } finally {
    await fs.writeFile(pdfUri.fsPath, validPdf);
  }
  previews.refresh(pdfUri);
  await eventually('valid PDF recovers after a load error', preview,
    result => result?.status === 'ready' && result.pages === 2 && !result.error);
  assertSourceFocus(savedSelection);
  console.log('PASS invalid PDF error and recovery without moving source focus');

  await vscode.window.tabGroups.close(pdfGroup.tabs.find(isPdf)!);
  assert.equal(preview(), undefined, 'closing the preview releases its session');
  await Promise.all([previews.open(pdfUri), previews.open(pdfUri)]);
  await eventually('closed PDF can be reopened', preview, result => result?.status === 'ready');
  assert.equal(vscode.window.tabGroups.all.flatMap(group => group.tabs).filter(isPdf).length, 1,
    'concurrent open requests reuse one preview');
  assertSourceFocus(savedSelection);
  console.log('PASS closing and concurrently reopening a PDF');

  await vscode.window.tabGroups.close(vscode.window.tabGroups.all.flatMap(group => group.tabs).find(isPdf)!);
  await vscode.commands.executeCommand('texquill.forwardSearch');
  await eventually('forward search opens a closed PDF and delivers its pending destination', preview,
    result => result?.status === 'ready' && !!result.navigation);
  assert.equal(previews.getSource(pdfUri)?.uri, mainUri.toString());
  assertSourceFocus(savedSelection);
  console.log('PASS forward search opens a new preview and preserves its source association');

  const sharedGroup = vscode.window.tabGroups.all.find(group => group.tabs.some(isPdf))!;
  const sharedEditor = await vscode.window.showTextDocument(document, { viewColumn: sharedGroup.viewColumn, preview: false });
  sharedEditor.selection = new vscode.Selection(5, 2, 5, 2);
  const sharedForward = await vscode.commands.executeCommand<Forward>('texquill.forwardSearch', mainUri);
  assert.equal(sharedForward!.locations[0].page, 2, 'forward search uses the focused source when it has multiple editors');
  await eventually('PDF moves beside the source when both shared a group', preview,
    result => result?.navigation?.page === 2 && result.renderedPages.includes(2));
  assert.equal(vscode.window.activeTextEditor, sharedEditor, 'revealing the PDF does not replace the focused source');
  assert.equal(vscode.window.tabGroups.activeTabGroup, sharedGroup);
  assert.ok(!sharedGroup.tabs.some(isPdf), 'PDF has moved to another group');
  await vscode.window.tabGroups.close(sharedGroup.activeTab!);
  await vscode.window.showTextDocument(document, { viewColumn: sourceGroup.viewColumn });
  await vscode.commands.executeCommand('texquill.openPdf');
  assertSourceFocus(savedSelection);
  console.log('PASS forward search preserves focus even when source and hidden PDF shared a group');

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
  assert.equal(preview()!.revision, 1, 'cancelled builds do not refresh the PDF');
  console.log('PASS cancellation terminates the compiler subprocess without refreshing the PDF');

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
