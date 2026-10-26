import * as assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import * as vscode from 'vscode';

async function waitFor(description: string, operation: () => unknown, accepts: (value: any) => boolean): Promise<void> {
  const deadline = Date.now() + 15_000;
  let last: unknown;
  while (Date.now() < deadline) {
    last = await operation();
    if (accepts(last)) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`${description}: ${JSON.stringify(last)}`);
}

export async function run(): Promise<void> {
  const file = vscode.Uri.file(path.join(process.env.TEXQUILL_TEST_WORKSPACE!, 'linked.tex'));
  const original = '\\begin{quote}\nBody\n\\end{quote}\n';
  await fs.writeFile(file.fsPath, original);
  await vscode.extensions.getExtension('asvetliakov.vscode-neovim')!.activate();
  const document = await vscode.workspace.openTextDocument(file);
  await vscode.window.showTextDocument(document);
  await vscode.extensions.getExtension('Jacky-Lzx.texquill')!.activate();
  const snapshot = vscode.commands.registerCommand('texquill.test.nvimSnapshot', data => console.log('Neovim snapshot:', JSON.stringify(data)));
  await new Promise(resolve => setTimeout(resolve, 1000));
  await vscode.commands.executeCommand('vscode-neovim.lua', "require('vscode').action('texquill.test.nvimSnapshot', {args={ {mode=vim.fn.mode(),name=vim.api.nvim_buf_get_name(0),lines=vim.api.nvim_buf_get_lines(0,0,-1,false)} }})");
  await vscode.commands.executeCommand('vscode-neovim.lua', 'vim.api.nvim_win_set_cursor(0, {1, 9})');
  await new Promise(resolve => setTimeout(resolve, 1000));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'i');
  await new Promise(resolve => setTimeout(resolve, 300));
  await vscode.commands.executeCommand('vscode-neovim.lua', "require('vscode').action('texquill.test.nvimSnapshot', {args={ {mode=vim.fn.mode(),name=vim.api.nvim_buf_get_name(0),cursor=vim.api.nvim_win_get_cursor(0)} }})");
  await vscode.commands.executeCommand('workbench.action.focusActiveEditorGroup');
  await vscode.commands.executeCommand('default:type', { text: 'x' });
  await waitFor('linked typing', () => document.getText(), text => text === '\\begin{quxote}\nBody\n\\end{quxote}\n');
  await vscode.commands.executeCommand('vscode-neovim.escape');
  await new Promise(resolve => setTimeout(resolve, 500));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await new Promise(resolve => setTimeout(resolve, 800));
  assert.equal(document.getText(), original, 'one Neovim undo restores both linked names');
  console.log('PASS one Neovim undo restores linked editing');
  await vscode.commands.executeCommand('vscode-neovim.lua', 'vim.api.nvim_win_set_cursor(0, {1, 9})');
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'x');
  await waitFor('normal mode linked delete', () => document.getText(), text => text === '\\begin{qute}\nBody\n\\end{qute}\n');
  await new Promise(resolve => setTimeout(resolve, 300));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await new Promise(resolve => setTimeout(resolve, 800));
  assert.equal(document.getText(), original, 'one Neovim undo restores a normal-mode linked edit');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', '<C-r>');
  await waitFor('redo restores both names', () => document.getText(), text => text === '\\begin{qute}\nBody\n\\end{qute}\n');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'j0rZ');
  await waitFor('independent body edit', () => document.getText(), text => text === '\\begin{qute}\nZody\n\\end{qute}\n');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await waitFor('body undo leaves linked edit intact', () => document.getText(), text => text === '\\begin{qute}\nBody\n\\end{qute}\n');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await waitFor('next undo restores the linked pair', () => document.getText(), text => text === original);
  console.log('PASS Neovim x/u/redo joins linked names while preserving independent body undo');
  await vscode.commands.executeCommand('vscode-neovim.lua', 'vim.api.nvim_win_set_cursor(0, {1, 9})');
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'ciw');
  await waitFor('ciw clears linked names', () => document.getText(), text => text === '\\begin{}\nBody\n\\end{}\n');
  await vscode.commands.executeCommand('default:type', { text: 'center' });
  await waitFor('ciw replacement links names', () => document.getText(), text => text === '\\begin{center}\nBody\n\\end{center}\n');
  await vscode.commands.executeCommand('vscode-neovim.escape');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await waitFor('ciw undo restores both names', () => document.getText(), text => text === original);
  console.log('PASS Neovim ciw replacement is one undo');
  await new Promise(resolve => setTimeout(resolve, 300));
  const unicode = '😀 \\begin{环境}\n\\begin{quote}Inner\\end{quote}\n\\end{环境}\n';
  const unicodeUri = vscode.Uri.file(path.join(process.env.TEXQUILL_TEST_WORKSPACE!, 'unicode.tex'));
  await fs.writeFile(unicodeUri.fsPath, unicode);
  const unicodeDocument = await vscode.workspace.openTextDocument(unicodeUri);
  await vscode.window.showTextDocument(unicodeDocument);
  await new Promise(resolve => setTimeout(resolve, 1000));
  await vscode.commands.executeCommand('vscode-neovim.lua', 'vim.api.nvim_win_set_cursor(0, {1, 12})');
  await new Promise(resolve => setTimeout(resolve, 800));
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'x');
  await waitFor('Unicode names link without changing nested environments', () => unicodeDocument.getText(),
    text => text === '😀 \\begin{境}\n\\begin{quote}Inner\\end{quote}\n\\end{境}\n');
  await vscode.commands.executeCommand('vscode-neovim.send-blocking', 'u');
  await waitFor('Unicode linked edit undoes once', () => unicodeDocument.getText(), text => text === unicode);
  console.log('PASS Neovim linked undo handles UTF-8/UTF-16 offsets and nested environments');
  snapshot.dispose();
}
