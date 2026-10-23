import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

const LANGUAGE_ID = 'definitively-asserted-daml';
const SERVER_SETTING = 'serverPath';

let client: LanguageClient | undefined;
let lifecycle = Promise.resolve();
let disposed = false;

function serverPath(): string {
  const configured = vscode.workspace.getConfiguration('definitivelyAssertedDaml').get<string>(SERVER_SETTING, 'daml-lsp').trim();
  return configured || 'daml-lsp';
}

function enqueue(action: () => Promise<void>): Promise<void> {
  lifecycle = lifecycle.then(action, action);
  return lifecycle;
}

async function stopClient(): Promise<void> {
  const previous = client;
  client = undefined;
  if (previous) {
    await previous.stop();
  }
}

async function startClient(): Promise<void> {
  if (disposed || !vscode.workspace.isTrusted || client) return;

  const executable = serverPath();
  const serverOptions: ServerOptions = {
    command: executable,
    args: [],
    options: { shell: false }
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: LANGUAGE_ID }, { scheme: 'untitled', language: LANGUAGE_ID }]
  };
  const next = new LanguageClient('definitively-asserted-daml', 'Definitively Asserted DAML', serverOptions, clientOptions);
  client = next;
  try {
    await next.start();
  } catch (error) {
    if (client === next) client = undefined;
    const detail = error instanceof Error ? error.message : String(error);
    const missing = /enoent|not found|cannot find/i.test(detail);
    const message = missing
      ? `Cannot start DAML language server: executable “${executable}” was not found. Install the daml-lsp binary from the private DAML v0.1.0 release, or set definitivelyAssertedDaml.serverPath to its absolute path in user settings.`
      : `Cannot start DAML language server (${executable}): ${detail}`;
    void vscode.window.showErrorMessage(message);
    throw error;
  }
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(vscode.workspace.onDidGrantWorkspaceTrust(() => {
    void enqueue(startClient).catch(() => undefined);
  }));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
    if (event.affectsConfiguration(`definitivelyAssertedDaml.${SERVER_SETTING}`) && vscode.workspace.isTrusted) {
      void enqueue(async () => {
        await stopClient();
        await startClient();
      }).catch(() => undefined);
    }
  }));
  context.subscriptions.push({ dispose: () => {
    disposed = true;
    void enqueue(stopClient);
  } });

  if (vscode.workspace.isTrusted) await enqueue(startClient).catch(() => undefined);
}

export async function deactivate(): Promise<void> {
  disposed = true;
  await enqueue(stopClient);
}
