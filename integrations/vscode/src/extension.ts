import * as path from 'node:path';
import * as vscode from 'vscode';
import { CloseAction, ErrorHandler, LanguageClient, State } from 'vscode-languageclient/node';
import { SessionLanguageClient } from './session-client';

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

type Configuration = { executable: string; errorHandler?: ErrorHandler };
type RetirementReason =
  | { kind: 'configuration' | 'deactivation' }
  | { kind: 'crash'; restart: boolean }
  | { kind: 'failure'; error: unknown };
type RetirementOutcome = { ok: true } | { ok: false; error: unknown };
type SessionPhase =
  | { kind: 'starting'; deadline: NodeJS.Timeout }
  | { kind: 'active' }
  | { kind: 'retiring'; reason: RetirementReason; outcome: Promise<RetirementOutcome> }
  | { kind: 'failed'; error: unknown; cleanupKnown: true }
  | { kind: 'failed'; error: unknown; cleanupKnown: false; cleanupError: unknown };
type OwnedSession = {
  configuration: Configuration;
  transport: SessionLanguageClient;
  dependencies?: DependencySession;
  startup: Promise<void>;
  phase: SessionPhase;
};
type Reconciliation = { pending: boolean; promise: Promise<void> };

let currentSession: OwnedSession | undefined;
let desired: Configuration | undefined;
let reconciliation: Reconciliation | undefined;
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
  private contextOrder = Promise.resolve();
  private readonly contextCalls = new Set<(error: Error) => void>();

  constructor(private readonly transport: LanguageClient, private readonly isCurrent: () => boolean) {
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
    return !this.retired && this.isCurrent() && this.transport.state === State.Running;
  }

  setContext(document: vscode.TextDocument, wire: DocumentContext): Promise<{ generation: number }> {
    if (!this.current()) return Promise.reject(new Error('The DAML language server session is inactive.'));
    const operation = this.contextOrder.then(() => this.applyContext(document, wire));
    this.contextOrder = operation.then(() => undefined, () => undefined);
    return new Promise((resolve, reject) => {
      this.contextCalls.add(reject);
      operation.then(value => {
        this.contextCalls.delete(reject);
        resolve(value);
      }, error => {
        this.contextCalls.delete(reject);
        reject(error);
      });
    });
  }

  private async applyContext(document: vscode.TextDocument, wire: DocumentContext): Promise<{ generation: number }> {
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
    const error = new Error('The DAML language server session retired before context selection completed.');
    for (const reject of this.contextCalls) reject(error);
    this.contextCalls.clear();
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

function ownsAdmission(owner: OwnedSession): boolean {
  return currentSession === owner && desired === owner.configuration && !disposed
    && vscode.workspace.isTrusted && (owner.phase.kind === 'starting' || owner.phase.kind === 'active');
}

function retireSession(owner: OwnedSession, reason: RetirementReason, stopping?: Promise<void>): void {
  if (owner.phase.kind === 'retiring' || owner.phase.kind === 'failed') return;
  const deadline = owner.phase.kind === 'starting' ? owner.phase.deadline : undefined;
  // Publish ownership before any fallible cleanup or re-entrant stop callback.
  let finish!: (outcome: RetirementOutcome) => void;
  const outcome = new Promise<RetirementOutcome>(resolve => { finish = resolve; });
  owner.phase = { kind: 'retiring', reason, outcome };
  clearTimeout(deadline);
  const failures: unknown[] = [];
  try {
    owner.transport.retire();
    owner.dependencies?.dispose();
    owner.dependencies = undefined;
  } catch (error) {
    // Keep the dependency owner reachable if any of its disposal failed.
    failures.push(error);
  }
  const complete = (transportOutcome: RetirementOutcome): void => {
    if (!transportOutcome.ok) failures.push(transportOutcome.error);
    finish(failures.length === 0 ? { ok: true } : {
      ok: false,
      error: failures.length === 1 ? failures[0] : new AggregateError(failures, 'DAML session cleanup failed.')
    });
  };
  try {
    (stopping ?? owner.transport.dispose()).then(
      () => complete({ ok: true }), error => complete({ ok: false, error })
    );
  } catch (error) {
    complete({ ok: false, error });
  }
}

function updateDesired(): Promise<void> {
  if (disposed) return reconcile();
  const executable = serverPath();
  if (desired?.executable !== executable) desired = { executable };
  if (currentSession && currentSession.configuration !== desired) {
    retireSession(currentSession, { kind: 'configuration' });
  }
  return reconcile();
}

function reconcile(): Promise<void> {
  if (reconciliation) {
    reconciliation.pending = true;
    return reconciliation.promise;
  }
  // Publish the request owner before its worker can reach any completion path.
  const request: Reconciliation = {
    pending: true,
    promise: Promise.resolve().then(() => drainReconciliation(request))
  };
  reconciliation = request;
  return request.promise;
}

async function drainReconciliation(request: Reconciliation): Promise<void> {
  let failure: { error: unknown } | undefined;
  for (;;) {
    request.pending = false;
    try {
      await reconcileSessions();
    } catch (error) {
      failure ??= { error };
    }
    if (request.pending) continue;
    // No await between the final pending check and releasing the shared owner.
    // A later callback must create a worker, not join an already-completed one.
    reconciliation = undefined;
    if (failure) throw failure.error;
    return;
  }
}

async function reconcileSessions(): Promise<void> {
  let startupFailure: { error: unknown } | undefined;
  for (;;) {
    const owner = currentSession;
    if (owner) {
      if (owner.phase.kind === 'starting') {
        try {
          await owner.startup;
        } catch (error) {
          if (owner.phase.kind === 'starting') {
            retireSession(owner, { kind: 'failure', error });
            const detail = error instanceof Error ? error.message : String(error);
            const executable = owner.configuration.executable;
            const message = /enoent|not found|cannot find/i.test(detail)
              ? `Cannot start DAML language server: executable “${executable}” was not found. Install daml-lsp from https://github.com/dprb-work/daml/releases, or set definitivelyAssertedDaml.serverPath to its absolute path in user settings.`
              : `Cannot start DAML language server (${executable}): ${detail}`;
            void vscode.window.showErrorMessage(message);
          }
        }
        if (owner.phase.kind === 'starting') {
          if (!ownsAdmission(owner)) {
            retireSession(owner, { kind: disposed ? 'deactivation' : 'configuration' });
          } else {
            clearTimeout(owner.phase.deadline);
            owner.phase = { kind: 'active' };
            owner.dependencies = new DependencySession(owner.transport, () => ownsAdmission(owner));
          }
        }
        continue;
      }
      if (owner.phase.kind === 'retiring') {
        const { reason, outcome } = owner.phase;
        const result = await outcome;
        if (!result.ok) {
          const error = reason.kind === 'failure' ? reason.error : result.error;
          owner.phase = { kind: 'failed', error, cleanupKnown: false, cleanupError: result.error };
          if (reason.kind === 'failure') reportFailure(result.error);
          throw error;
        }
        if (reason.kind === 'failure') {
          owner.phase = { kind: 'failed', error: reason.error, cleanupKnown: true };
          startupFailure ??= { error: reason.error };
          continue;
        }
        if (reason.kind === 'crash' && !reason.restart) {
          owner.phase = { kind: 'failed', error: new Error('The DAML language server stopped; automatic restart is not permitted.'), cleanupKnown: true };
          continue;
        }
        currentSession = undefined;
        continue;
      }
      if (owner.phase.kind === 'failed') {
        if (!owner.phase.cleanupKnown) throw owner.phase.error;
        if (disposed || desired !== owner.configuration) {
          currentSession = undefined;
          continue;
        }
        if (startupFailure) throw startupFailure.error;
        return;
      }
      if (!ownsAdmission(owner)) {
        retireSession(owner, { kind: disposed ? 'deactivation' : 'configuration' });
        continue;
      }
      if (startupFailure) throw startupFailure.error;
      return;
    }
    if (disposed || !vscode.workspace.isTrusted || !desired) {
      if (startupFailure) throw startupFailure.error;
      return;
    }
    const configuration = desired;
    let nextOwner: OwnedSession;
    const transport = new SessionLanguageClient(
      'definitively-asserted-daml', 'Definitively Asserted DAML',
      { command: configuration.executable, args: [], options: { shell: false } },
      { documentSelector: [{ scheme: 'file', language: LANGUAGE_ID }, { scheme: 'untitled', language: LANGUAGE_ID }] },
      {
        isCurrent: () => ownsAdmission(nextOwner),
        errorHandler: configuration.errorHandler,
        onClosed: decision => {
          if (currentSession !== nextOwner || nextOwner.phase.kind === 'failed' || nextOwner.phase.kind === 'retiring') return;
          retireSession(nextOwner, { kind: 'crash', restart: decision.action === CloseAction.Restart });
          void reconcile().catch(reportFailure);
        },
        onStop: stopping => {
          if (currentSession !== nextOwner || nextOwner.phase.kind !== 'active') return;
          retireSession(nextOwner, { kind: 'crash', restart: false }, stopping);
          void reconcile().catch(reportFailure);
        }
      }
    );
    configuration.errorHandler = transport.restartPolicy;
    const deadline = setTimeout(() => {
      if (currentSession === nextOwner && nextOwner.phase.kind === 'starting') {
        retireSession(nextOwner, { kind: 'failure', error: new Error('DAML language server startup timed out after the 30-second deadline.') });
      }
    }, 30_000);
    nextOwner = {
      configuration, transport, phase: { kind: 'starting', deadline },
      startup: Promise.resolve().then(() => transport.start())
    };
    currentSession = nextOwner;
  }
}

async function selectContext(kind: ContextKind, args: unknown): Promise<{ generation: number } | undefined> {
  try {
    if (disposed || !vscode.workspace.isTrusted) throw new Error('DAML context selection requires a trusted workspace.');
    // Capture identity before widgets or startup waits. Never replay on a replacement.
    const owner = currentSession;
    if (!owner || !ownsAdmission(owner)) throw new Error('The DAML language server is not active.');
    const target = args === undefined ? await chooseContext(kind) : contextArguments(kind, args);
    if (!target) return undefined; // Public selection widget was cancelled.
    await owner.startup;
    if (!ownsAdmission(owner) || owner.phase.kind !== 'active' || !owner.dependencies) {
      throw new Error('The DAML language server session changed before context selection.');
    }
    return await owner.dependencies.setContext(target.document, target.wire);
  } catch (error) {
    reportFailure(error);
    throw error;
  }
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  if (disposed) throw new Error('The DAML extension has been deactivated.');
  for (const [command, kind] of [
    ['useConfiguration', 'configuration'], ['useDefinition', 'definition'], ['useSyntax', 'syntax']
  ] as const) {
    context.subscriptions.push(vscode.commands.registerCommand(
      `definitivelyAssertedDaml.${command}`, (args: unknown) => selectContext(kind, args)
    ));
  }
  context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(document => currentSession?.dependencies?.close(document)));
  context.subscriptions.push(vscode.workspace.onDidGrantWorkspaceTrust(() => {
    void updateDesired().catch(reportFailure);
  }));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(event => {
    if (event.affectsConfiguration(`definitivelyAssertedDaml.${SERVER_SETTING}`) && vscode.workspace.isTrusted) {
      void updateDesired().catch(reportFailure);
    }
  }));
  context.subscriptions.push({ dispose: () => { void deactivate().catch(reportFailure); } });
  if (vscode.workspace.isTrusted) await updateDesired();
}

export async function deactivate(): Promise<void> {
  disposed = true;
  if (currentSession) retireSession(currentSession, { kind: 'deactivation' });
  await reconcile();
}
