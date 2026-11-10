import * as path from 'node:path';
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions, State } from 'vscode-languageclient/node';

const LANGUAGE_ID = 'definitively-asserted-daml';
const SERVER_SETTING = 'serverPath';
const MAX_DEPENDENCIES = 10_000;
const MAX_URI_LENGTH = 32_768;
const MAX_DEPENDENCY_BYTES = 8 * 1024 * 1024;

type ContextKind = 'configuration' | 'definition' | 'syntax';
type DocumentContext =
  | { uri: string; kind: 'configuration'; bundleRoot?: string; expected?: string }
  | { uri: string; kind: 'definition'; bundleRoot?: string }
  | { uri: string; kind: 'syntax' };
type Selection = { document: vscode.TextDocument; generation: number; files: Set<string> };
type DependencySnapshot = { version: number; generation: number; files: Map<string, vscode.Uri> };
type PendingContext = {
  document: vscode.TextDocument;
  snapshot?: DependencySnapshot;
  failure?: { generation: number; error: unknown };
};
type SharedWatcher = { watcher: vscode.FileSystemWatcher; documents: Set<string> };

let client: LanguageClient | undefined;
let clientStateSubscription: vscode.Disposable | undefined;
let session: DependencySession | undefined;
let lifecycle = Promise.resolve();
let disposed = false;

function record(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object.`);
  }
  return value as Record<string, unknown>;
}

function checkFields(value: Record<string, unknown>, allowed: readonly string[], label: string): void {
  if (Object.keys(value).some(key => !allowed.includes(key))) {
    throw new Error(`${label} contains an unknown field.`);
  }
}

function uriString(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.length === 0 || value.length > MAX_URI_LENGTH) {
    throw new Error(`${label} must be a nonempty URI of at most ${MAX_URI_LENGTH} characters.`);
  }
  return value;
}

function localFileUri(value: unknown, label: string): vscode.Uri {
  const text = uriString(value, label);
  if (!text.startsWith('file://') || /[?#\u0000-\u0020\u007f]/.test(text) || /%(?![0-9a-f]{2})/i.test(text)) {
    throw new Error(`${label} must be an absolute local file URI without a query or fragment.`);
  }
  // VS Code's URI parser is intentionally forgiving; validate percent encoding
  // before allowing its filesystem representation to own a watcher.
  decodeURIComponent(text);
  const uri = vscode.Uri.parse(text, true);
  const localPath = uri.with({ authority: '' }).fsPath;
  if (uri.scheme !== 'file' || (uri.authority !== '' && uri.authority.toLowerCase() !== 'localhost')
      || !uri.path.startsWith('/') || uri.path.includes('\0') || !path.isAbsolute(localPath)) {
    throw new Error(`${label} must identify an absolute local file path.`);
  }
  const local = vscode.Uri.file(localPath);
  if (local.authority !== '') throw new Error(`${label} must not identify a network share.`);
  return local;
}

function generation(value: unknown): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 1 || value > 0xffff_ffff) {
    throw new Error('DAML context generation must be a nonzero unsigned 32-bit integer.');
  }
  return value;
}

function liveDocument(document: vscode.TextDocument): boolean {
  return !document.isClosed && document.languageId === LANGUAGE_ID
    && vscode.workspace.textDocuments.includes(document);
}

function documentFromUri(value: unknown): vscode.TextDocument {
  const uri = vscode.Uri.parse(uriString(value, 'Document URI'), true);
  if (uri.scheme === 'file') localFileUri(value, 'Document URI');
  else if (uri.scheme !== 'untitled') throw new Error('DAML context requires a local file or untitled document.');
  const document = vscode.workspace.textDocuments.find(candidate => candidate.uri.toString() === uri.toString());
  if (!document || !liveDocument(document)) throw new Error('DAML context requires an open DAML document.');
  return document;
}

function contextArguments(kind: ContextKind, value: unknown): { document: vscode.TextDocument; wire: DocumentContext } {
  const args = record(value, 'DAML context arguments');
  const allowed = kind === 'configuration' ? ['uri', 'bundleRoot', 'expected']
    : kind === 'definition' ? ['uri', 'bundleRoot'] : ['uri'];
  checkFields(args, allowed, 'DAML context arguments');
  const document = documentFromUri(args.uri);
  const uri = document.uri.toString();
  if (kind === 'syntax') return { document, wire: { uri, kind } };
  if (kind === 'definition' && document.uri.scheme !== 'file') {
    throw new Error('Native definition context requires a local file identity. Save the document first.');
  }
  const bundleRoot = 'bundleRoot' in args ? localFileUri(args.bundleRoot, 'Bundle root').toString() : undefined;
  if (document.uri.scheme === 'untitled' && bundleRoot === undefined) {
    throw new Error('An untitled configuration requires an explicitly selected local bundle folder.');
  }
  const bundle = bundleRoot === undefined ? {} : { bundleRoot };
  if (kind === 'definition') return { document, wire: { uri, kind, ...bundle } };
  if ('expected' in args && (typeof args.expected !== 'string' || args.expected.trim().length === 0)) {
    throw new Error('The expected configuration root must be a nonempty native type name.');
  }
  const expected = typeof args.expected === 'string' ? { expected: args.expected } : {};
  return { document, wire: { uri, kind, ...bundle, ...expected } };
}

async function chooseContext(kind: ContextKind): Promise<{ document: vscode.TextDocument; wire: DocumentContext } | undefined> {
  const document = vscode.window.activeTextEditor?.document;
  if (!document || !liveDocument(document)) throw new Error('Open a DAML document to select its context.');
  const args: Record<string, unknown> = { uri: document.uri.toString() };
  // Admit the document before showing widgets (notably, never invent an
  // identity for an untitled definition).
  if (kind === 'syntax') return contextArguments(kind, args);
  if (kind === 'definition' && document.uri.scheme !== 'file') {
    throw new Error('Native definition context requires a local file identity. Save the document first.');
  }
  if (document.uri.scheme === 'file') localFileUri(args.uri, 'Document URI');
  else if (document.uri.scheme !== 'untitled') throw new Error('DAML context requires a local file or untitled document.');
  let selectBundle = document.uri.scheme === 'untitled';
  if (!selectBundle) {
    const choice = await vscode.window.showQuickPick([
      { label: 'Discover from this document', description: 'Use its nearest ancestor DAML bundle', selectBundle: false },
      { label: 'Select a bundle folder', description: 'Choose an explicit local DAML bundle', selectBundle: true }
    ], { title: 'DAML context: bundle', placeHolder: 'Choose how this document selects its bundle' });
    if (!choice) return undefined;
    selectBundle = choice.selectBundle;
  }
  if (selectBundle) {
    const folders = await vscode.window.showOpenDialog({
      title: 'Select DAML bundle folder', openLabel: 'Use bundle',
      canSelectFiles: false, canSelectFolders: true, canSelectMany: false
    });
    if (!folders || folders.length === 0) return undefined;
    args.bundleRoot = localFileUri(folders[0].toString(), 'Bundle root').toString();
  }
  if (kind === 'configuration') {
    const expected = await vscode.window.showInputBox({
      title: 'DAML context: configuration root',
      prompt: 'Optional native type name; leave blank to use the document’s root assertion',
      placeHolder: 'app.Config', ignoreFocusOut: true
    });
    if (expected === undefined) return undefined;
    if (expected.trim().length > 0) args.expected = expected.trim();
  }
  return contextArguments(kind, args);
}

function dependencyFiles(value: unknown): Map<string, vscode.Uri> {
  if (!Array.isArray(value) || value.length > MAX_DEPENDENCIES) {
    throw new Error(`DAML dependencies must be an array of at most ${MAX_DEPENDENCIES} local file URIs.`);
  }
  let bytes = 0;
  const files = new Map<string, vscode.Uri>();
  for (const item of value) {
    const text = uriString(item, 'Dependency URI');
    bytes += Buffer.byteLength(text, 'utf8');
    if (bytes > MAX_DEPENDENCY_BYTES) throw new Error('DAML dependency URI data exceeds 8 MiB.');
    const uri = localFileUri(text, 'Dependency URI');
    files.set(uri.toString(), uri);
  }
  return files;
}

class DependencySession implements vscode.Disposable {
  private readonly selections = new Map<string, Selection>();
  private readonly pending = new Map<string, PendingContext>();
  private readonly watchers = new Map<string, SharedWatcher>();
  private readonly notification: vscode.Disposable;
  private lastGeneration = 0;
  private retired = false;

  constructor(private readonly transport: LanguageClient) {
    this.notification = transport.onNotification('daml/dependencies', (value: unknown) => {
      if (!this.current()) return;
      try {
        this.receive(value);
      } catch (error) {
        transport.error('Cannot update DAML dependency watchers.', error, false);
        reportFailure(error);
      }
    });
  }

  private current(): boolean {
    return !this.retired && !disposed && vscode.workspace.isTrusted
      && session === this && client === this.transport && this.transport.state === State.Running;
  }

  async setContext(document: vscode.TextDocument, wire: DocumentContext): Promise<{ generation: number }> {
    if (!this.current() || !liveDocument(document)) throw new Error('The DAML document or language server is no longer active.');
    const key = document.uri.toString();
    const pending: PendingContext = { document };
    this.pending.set(key, pending);
    try {
      const result = record(await this.transport.sendRequest<unknown>('workspace/executeCommand', {
        command: 'daml.setContext', arguments: [wire]
      }), 'DAML context response');
      checkFields(result, ['generation'], 'DAML context response');
      const acknowledged = generation(result.generation);
      if (acknowledged <= this.lastGeneration) throw new Error('The DAML server reused an old context generation.');
      this.lastGeneration = acknowledged;
      if (!this.current() || !liveDocument(document) || this.pending.get(key) !== pending) {
        throw new Error('The DAML document or language server closed before context selection completed.');
      }
      if (wire.kind === 'syntax') {
        this.close(document);
      } else {
        const previous = this.selections.get(key);
        const selection: Selection = {
          document, generation: acknowledged, files: previous?.files ?? new Set()
        };
        // The server has accepted this identity even if its watcher update
        // fails. Retain prior memberships until a complete new update commits,
        // so a later valid notification can recover without a context retry.
        this.selections.set(key, selection);
        if (pending.failure?.generation === acknowledged) throw pending.failure.error;
        const snapshot = pending.snapshot;
        if (snapshot?.generation === acknowledged && snapshot.version === document.version) {
          this.replaceFiles(key, selection, snapshot.files);
        }
      }
      return { generation: acknowledged };
    } finally {
      if (this.pending.get(key) === pending) this.pending.delete(key);
    }
  }

  private receive(value: unknown): void {
    const data = record(value, 'DAML dependency notification');
    const key = uriString(data.uri, 'Dependency document URI');
    const selected = this.selections.get(key);
    const pending = this.pending.get(key);
    const document = pending?.document ?? selected?.document;
    if (!document || !liveDocument(document)) return;
    if (typeof data.version !== 'number' || !Number.isSafeInteger(data.version)) {
      throw new Error('DAML dependency version must be an integer.');
    }
    if (data.version !== document.version) return;
    const identity = generation(data.generation);
    const matchesSelection = selected?.document === document && selected.generation === identity;
    const matchesPending = pending !== undefined && identity > this.lastGeneration;
    if (!matchesSelection && !matchesPending) return;
    if (matchesPending && identity < Math.max(pending.snapshot?.generation ?? 0, pending.failure?.generation ?? 0)) return;
    try {
      checkFields(data, ['uri', 'version', 'generation', 'files'], 'DAML dependency notification');
      const snapshot = { version: data.version, generation: identity, files: dependencyFiles(data.files) };
      if (matchesSelection) {
        this.replaceFiles(key, selected, snapshot.files);
      } else if (pending) {
        // One newest-generation snapshot bounds pending metadata; the response
        // must acknowledge this exact identity before it can install watchers.
        pending.snapshot = snapshot;
        pending.failure = undefined;
      }
    } catch (error) {
      if (matchesPending && pending) {
        pending.snapshot = undefined;
        pending.failure = { generation: identity, error };
      }
      throw error;
    }
  }

  private createWatcher(key: string, uri: vscode.Uri): SharedWatcher {
    const basename = path.basename(uri.fsPath).replace(/[*?[\]{}\\]/g, character => `[${character}]`);
    const watcher = vscode.workspace.createFileSystemWatcher(
      new vscode.RelativePattern(vscode.Uri.file(path.dirname(uri.fsPath)), basename)
    );
    const entry: SharedWatcher = { watcher, documents: new Set() };
    // The exact-file RelativePattern owns event matching, including host casing.
    const forward = (type: 1 | 2 | 3): void => {
      if (!this.current() || this.watchers.get(key) !== entry || entry.documents.size === 0) return;
      void this.transport.sendNotification('workspace/didChangeWatchedFiles', {
        changes: [{ uri: key, type }]
      }).catch(error => {
        if (this.current()) {
          this.transport.error('Cannot forward a DAML dependency change.', error, false);
          reportFailure(error);
        }
      });
    };
    try {
      watcher.onDidCreate(() => forward(1));
      watcher.onDidChange(() => forward(2));
      watcher.onDidDelete(() => forward(3));
      return entry;
    } catch (error) {
      watcher.dispose();
      throw error;
    }
  }

  private replaceFiles(key: string, selection: Selection, files: Map<string, vscode.Uri>): void {
    let additions = 0;
    for (const file of files.keys()) if (!this.watchers.has(file)) additions++;
    if (this.watchers.size + additions > MAX_DEPENDENCIES) {
      throw new Error(`DAML dependency watching exceeds ${MAX_DEPENDENCIES} shared files, including pending replacements.`);
    }
    const tentative = new Map<string, SharedWatcher>();
    try {
      for (const [file, uri] of files) {
        if (!this.watchers.has(file)) tentative.set(file, this.createWatcher(file, uri));
      }
    } catch (error) {
      for (const entry of tentative.values()) entry.watcher.dispose();
      throw error;
    }
    for (const [file, entry] of tentative) this.watchers.set(file, entry);
    for (const file of files.keys()) this.watchers.get(file)?.documents.add(key);
    for (const file of selection.files) {
      if (!files.has(file)) this.releaseFile(file, key);
    }
    selection.files = new Set(files.keys());
  }

  private releaseFile(file: string, key: string): void {
    const entry = this.watchers.get(file);
    if (!entry) return;
    entry.documents.delete(key);
    if (entry.documents.size === 0) {
      this.watchers.delete(file);
      entry.watcher.dispose();
    }
  }

  close(document: vscode.TextDocument): void {
    const key = document.uri.toString();
    if (this.pending.get(key)?.document === document) this.pending.delete(key);
    const selection = this.selections.get(key);
    if (selection?.document !== document) return;
    this.selections.delete(key);
    for (const file of selection.files) this.releaseFile(file, key);
  }

  dispose(): void {
    this.retired = true;
    this.notification.dispose();
    this.pending.clear();
    this.selections.clear();
    for (const entry of this.watchers.values()) entry.watcher.dispose();
    this.watchers.clear();
  }
}

function reportFailure(error: unknown): void {
  const detail = error instanceof Error ? error.message : String(error);
  void vscode.window.showErrorMessage(`DAML: ${detail}`);
}

function serverPath(): string {
  const configured = vscode.workspace.getConfiguration('definitivelyAssertedDaml').get<string>(SERVER_SETTING, 'daml-lsp').trim();
  return configured || 'daml-lsp';
}

function enqueue<T>(action: () => Promise<T>): Promise<T> {
  const result = lifecycle.then(action, action);
  lifecycle = result.then(() => undefined, () => undefined);
  return result;
}

function clearSession(): void {
  session?.dispose();
  session = undefined;
}

async function stopClient(): Promise<void> {
  const previous = client;
  client = undefined;
  clearSession();
  clientStateSubscription?.dispose();
  clientStateSubscription = undefined;
  if (previous) await previous.dispose();
}

async function startClient(): Promise<void> {
  if (disposed || !vscode.workspace.isTrusted || client) return;
  const executable = serverPath();
  const serverOptions: ServerOptions = {
    command: executable, args: [], options: { shell: false }
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: LANGUAGE_ID }, { scheme: 'untitled', language: LANGUAGE_ID }]
  };
  const next = new LanguageClient('definitively-asserted-daml', 'Definitively Asserted DAML', serverOptions, clientOptions);
  client = next;
  clientStateSubscription = next.onDidChangeState(event => {
    if (client !== next) return;
    clearSession();
    if (!disposed && vscode.workspace.isTrusted && event.newState === State.Running) {
      session = new DependencySession(next);
    }
  });
  try {
    await next.start();
  } catch (error) {
    if (client === next) {
      client = undefined;
      clearSession();
      clientStateSubscription?.dispose();
      clientStateSubscription = undefined;
    }
    const detail = error instanceof Error ? error.message : String(error);
    const missing = /enoent|not found|cannot find/i.test(detail);
    const message = missing
      ? `Cannot start DAML language server: executable “${executable}” was not found. Install daml-lsp from https://github.com/dprb-work/daml/releases, or set definitivelyAssertedDaml.serverPath to its absolute path in user settings.`
      : `Cannot start DAML language server (${executable}): ${detail}`;
    void vscode.window.showErrorMessage(message);
    throw error;
  }
}

async function selectContext(kind: ContextKind, args: unknown): Promise<{ generation: number } | undefined> {
  try {
    if (disposed || !vscode.workspace.isTrusted) throw new Error('DAML context selection requires a trusted workspace.');
    const target = args === undefined ? await chooseContext(kind) : contextArguments(kind, args);
    if (!target) return undefined; // Public selection widget was cancelled.
    return await enqueue(async () => {
      if (disposed || !vscode.workspace.isTrusted) throw new Error('DAML context selection requires a trusted workspace.');
      await startClient();
      if (!session) throw new Error('The DAML language server is not running.');
      return session.setContext(target.document, target.wire);
    });
  } catch (error) {
    reportFailure(error);
    throw error;
  }
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  disposed = false;
  for (const [command, kind] of [
    ['useConfiguration', 'configuration'], ['useDefinition', 'definition'], ['useSyntax', 'syntax']
  ] as const) {
    context.subscriptions.push(vscode.commands.registerCommand(
      `definitivelyAssertedDaml.${command}`, (args: unknown) => selectContext(kind, args)
    ));
  }
  context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(document => session?.close(document)));
  context.subscriptions.push(vscode.workspace.onDidGrantWorkspaceTrust(() => {
    void enqueue(startClient).catch(reportFailure);
  }));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
    if (event.affectsConfiguration(`definitivelyAssertedDaml.${SERVER_SETTING}`) && vscode.workspace.isTrusted) {
      void enqueue(async () => {
        await stopClient();
        await startClient();
      }).catch(reportFailure);
    }
  }));
  context.subscriptions.push({ dispose: () => {
    disposed = true;
    clearSession();
    void enqueue(stopClient).catch(reportFailure);
  } });
  if (vscode.workspace.isTrusted) await enqueue(startClient);
}

export async function deactivate(): Promise<void> {
  disposed = true;
  clearSession();
  await enqueue(stopClient);
}
