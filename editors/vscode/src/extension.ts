import * as vscode from 'vscode';
import { LanguageClient, TransportKind } from 'vscode-languageclient/node';
import { PdfPreviewProvider } from './preview/provider';
import { SyncTexNavigation } from './synctex';
import { type InverseSearch } from './preview/protocol';

const languages = ['latex', 'tex', 'plaintex', 'bibtex', 'bib', 'bst', 'bibtex-style'];
type BuildStatus = {
  status: string;
  stale?: boolean;
  message?: string;
  pdfUri?: string;
  rootUri?: string;
};

class LatexExtension implements vscode.Disposable {
  private client: LanguageClient | undefined;
  private pending: Promise<void> = Promise.resolve();
  private disposed = false;

  constructor(private readonly output: vscode.OutputChannel, private readonly previews: PdfPreviewProvider) {}

  // Serialize restarts, settings updates, and shutdown so clients never overlap.
  private enqueue(operation: () => Promise<void>): Promise<void> {
    const result = this.pending.then(operation);
    this.pending = result.catch(() => {});
    return result;
  }

  restart(): Promise<void> {
    return this.enqueue(async () => {
      await this.stopClient();
      if (this.disposed) return;
      const config = vscode.workspace.getConfiguration('texquill');
      const command = config.get<string>('serverPath', 'texquill').trim();
      if (!command) throw new Error('Set texquill.serverPath to the texquill executable.');
      const client = new LanguageClient('texquill', 'TeXquill', {
        command,
        args: ['--stdio'],
        transport: TransportKind.stdio,
        options: { env: { ...process.env, ...config.get<Record<string, string>>('serverEnvironment', {}) } },
      }, {
        documentSelector: languages.flatMap(language => [
          { scheme: 'file', language },
          { scheme: 'untitled', language },
        ]),
        initializationOptions: config.get<object>('settings', {}),
        outputChannel: this.output,
      });
      this.client = client;
      client.onNotification('latex/buildStarted', (result: unknown) => {
        this.output.appendLine(`Build started: ${JSON.stringify(result)}`);
      });
      client.onNotification('latex/buildFinished', (result: BuildStatus) => {
        this.output.appendLine(`Build finished: ${JSON.stringify(result, null, 2)}`);
        if (result.status === 'success' && !result.stale && result.pdfUri) {
          this.previews.refresh(vscode.Uri.parse(result.pdfUri, true));
        }
        if (!result.stale && (result.status === 'error' || result.status === 'timeout')) {
          void vscode.window.showErrorMessage(`TeXquill: ${result.message ?? result.status}`);
        }
      });
      try {
        await client.start();
      } catch (error) {
        await this.stopClient();
        throw new Error(`Could not start ${command}. Check texquill.serverPath and PATH, then run “TeXquill: Restart Server”. ${String(error)}`);
      }
    });
  }

  updateSettings(): Promise<void> {
    return this.enqueue(async () => {
      if (this.client?.isRunning()) {
        await this.client.sendNotification('workspace/didChangeConfiguration', {
          settings: vscode.workspace.getConfiguration('texquill').get<object>('settings', {}),
        });
      }
    });
  }

  async execute<T>(command: string, args: unknown[]): Promise<T> {
    await this.pending;
    if (!this.client?.isRunning()) {
      throw new Error('The server is not running. Check the TeXquill output and run “TeXquill: Restart Server”.');
    }
    return this.client.sendRequest<T>('workspace/executeCommand', { command, arguments: args });
  }

  private async stopClient(): Promise<void> {
    const client = this.client;
    this.client = undefined;
    if (!client) return;
    const running = client.isRunning();
    try {
      await client.dispose(10_000);
    } catch (error) {
      // vscode-languageclient rejects stop() in its StartFailed state. Its
      // Node client still cleans up the spawned process in a finally block.
      // Preserve the original startup error and allow a fresh client to start.
      if (running) throw error;
      this.output.appendLine(`Server cleanup: ${String(error)}`);
    }
  }

  shutdown(): Promise<void> {
    this.disposed = true;
    return this.enqueue(() => this.stopClient());
  }

  dispose(): void {
    void this.shutdown().catch(error => this.output.appendLine(String(error)));
  }
}

let extension: LatexExtension | undefined;

function documentUri(argument?: vscode.Uri): string {
  const uri = argument ?? vscode.window.activeTextEditor?.document.uri;
  if (!uri || uri.scheme !== 'file') {
    throw new Error('Open and save a LaTeX project file first.');
  }
  return uri.toString();
}

export async function activate(context: vscode.ExtensionContext): Promise<{ previews: PdfPreviewProvider }> {
  const output = vscode.window.createOutputChannel('TeXquill');
  const previews = new PdfPreviewProvider(context, output);
  const controller = new LatexExtension(output, previews);
  const navigation = new SyncTexNavigation((command, args) => controller.execute(command, args), previews);
  extension = controller;
  context.subscriptions.push(output, controller, previews);

  function register(name: string, action: (uri?: vscode.Uri) => unknown) {
    context.subscriptions.push(vscode.commands.registerCommand(`texquill.${name}`, async (uri?: vscode.Uri) => {
      try {
        return await action(uri);
      } catch (error) {
        output.appendLine(String(error));
        void vscode.window.showErrorMessage(`TeXquill: ${error instanceof Error ? error.message : String(error)}`);
        // Preserve rejection for callers of executeCommand, including tests.
        throw error;
      }
    }));
  }

  for (const command of ['build', 'rebuild', 'cancelBuild', 'buildStatus']) {
    register(command, async uri => {
      const args = command === 'cancelBuild' && !uri && !vscode.window.activeTextEditor
        ? [] : [documentUri(uri)];
      const result = await controller.execute(`latex.${command}`, args);
      output.appendLine(`${command}: ${JSON.stringify(result, null, 2)}`);
      if (command === 'buildStatus') output.show(true);
      return result;
    });
  }
  register('openPdf', async uri => {
    const source = { uri: documentUri(uri), column: vscode.window.activeTextEditor?.viewColumn };
    const result = await controller.execute<BuildStatus>('latex.buildStatus', [source.uri]);
    if (result.status !== 'success' || result.stale || !result.pdfUri) {
      throw new Error('Build this project successfully before opening its PDF.');
    }
    const pdf = vscode.Uri.parse(result.pdfUri, true);
    if (pdf.scheme !== 'file') throw new Error('The build did not return a local PDF.');
    await previews.open(pdf, true, { ...source, uri: result.rootUri ?? source.uri });
  });
  register('forwardSearch', uri => navigation.forward(uri));
  context.subscriptions.push(vscode.commands.registerCommand('texquill.inverseSearch', async (point: InverseSearch) => {
    try { return await navigation.inverse(point); }
    catch (error) {
      output.appendLine(String(error));
      void vscode.window.showErrorMessage(`TeXquill: ${error instanceof Error ? error.message : String(error)}`);
      throw error;
    }
  }), previews.onDidRequestInverseSearch(point => {
    void vscode.commands.executeCommand('texquill.inverseSearch', point).then(undefined, () => {});
  }));
  register('restart', () => controller.restart());
  register('showOutput', () => output.show(true));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
    const changedProcess = event.affectsConfiguration('texquill.serverPath')
      || event.affectsConfiguration('texquill.serverEnvironment');
    const operation = changedProcess ? controller.restart()
      : event.affectsConfiguration('texquill.settings') ? controller.updateSettings() : undefined;
    void operation?.catch(error => {
      output.appendLine(String(error));
      void vscode.window.showErrorMessage(`TeXquill: ${String(error)}`);
    });
  }));

  // Keep commands available after a startup failure so the user can fix the path
  // and restart without having to reload the extension host.
  try {
    await controller.restart();
  } catch (error) {
    output.appendLine(String(error));
    void vscode.window.showErrorMessage(`TeXquill: ${String(error)}`);
  }
  return { previews };
}

export async function deactivate(): Promise<void> {
  await extension?.shutdown();
  extension = undefined;
}
