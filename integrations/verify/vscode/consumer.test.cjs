const assert = require('node:assert/strict');
const { PassThrough } = require('node:stream');
const cp = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const { createRequire } = require('node:module');
const vscode = require('vscode');
const observeVscode = require('./vscode-observer.cjs');
const submittedSemanticProvider = require('./consumer-inflight.cjs');
const packagePath = path.join(__dirname, '../../vscode/package.json');
const dependencyRequire = createRequire(packagePath);
const rpc = dependencyRequire('vscode-jsonrpc/node');
function deferred() { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; }
function observed(promise) { return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason })); }
async function bounded(promise, label) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`No consumer observation: ${label}`)), 5000); })]); }
  finally { clearTimeout(timer); }
}
function fixture(SessionLanguageClient, sdk, middleware = {}) {
  const outbound = [];
  const input = new PassThrough();
  const output = new PassThrough();
  const shutdown = deferred();
  const acknowledge = deferred();
  const closed = deferred();
  const stopped = deferred();
  const stopping = [];
  const server = rpc.createMessageConnection(new rpc.StreamMessageReader(output), new rpc.StreamMessageWriter(input));
  server.onRequest('initialize', () => ({ capabilities: {
    textDocumentSync: 1, documentFormattingProvider: true,
    semanticTokensProvider: { legend: { tokenTypes: ['string'], tokenModifiers: [] }, full: true },
  } }));
  server.onRequest('shutdown', () => { shutdown.resolve(); return acknowledge.promise; });
  server.onRequest('textDocument/semanticTokens/full', () => ({ data: [] }));
  server.onRequest('textDocument/formatting', () => []);
  server.listen();
  const stock = new rpc.StreamMessageWriter(output);
  const client = new SessionLanguageClient('consumer-check', 'DAML consumer check', () => Promise.resolve({
    reader: new rpc.StreamMessageReader(input), writer: {
      onError: stock.onError, onClose: stock.onClose,
      write(message) { outbound.push(message); return stock.write(message); },
      end() { stock.end(); }, dispose() { stock.dispose(); },
    },
  }), { documentSelector: [{ scheme: 'untitled', language: 'plaintext' }], middleware }, {
    isCurrent: () => true,
    errorHandler: { error: () => ({ action: sdk.ErrorAction.Continue }), closed: () => ({ action: sdk.CloseAction.DoNotRestart, handled: true }) },
    onClosed(decision) { closed.resolve(decision); }, onStop(promise) { stopping.push(observed(promise)); },
  });
  const subscription = client.onDidChangeState(event => { if (event.newState === sdk.State.Stopped) stopped.resolve(); });
  return { client, outbound, server, shutdown: shutdown.promise, acknowledge, closed: closed.promise, stopped: stopped.promise,
    closeUnexpectedly() { server.end(); },
    async close() { acknowledge.resolve(null); await client.dispose(100).catch(() => {}); subscription.dispose(); server.dispose(); input.destroy(); output.destroy(); await Promise.all(stopping); },
  };
}
async function suspendedProvider(SessionLanguageClient, sdk, kind) {
  const entered = deferred();
  const resume = deferred();
  const middleware = kind === 'semantic' ? {
    async provideDocumentSemanticTokens(document, token, next) { entered.resolve(); await resume.promise; return next(document, token); },
  } : {
    async provideDocumentFormattingEdits(document, options, token, next) { entered.resolve(); await resume.promise; return next(document, options, token); },
  };
  const f = fixture(SessionLanguageClient, sdk, middleware);
  try {
    await bounded(f.client.start(), `${kind} client start`);
    const document = await vscode.workspace.openTextDocument({ language: 'plaintext', content: '{}' });
    const pending = kind === 'semantic'
      ? observed(f.client.getFeature(sdk.SemanticTokensRegistrationType.type.method).getProvider(document).full.provideDocumentSemanticTokens(document, rpc.CancellationToken.None))
      : observed(f.client.getFeature(sdk.DocumentFormattingRequest.type.method).getProvider(document).provideDocumentFormattingEdits(document, { tabSize: 2, insertSpaces: true }, rpc.CancellationToken.None));
    await bounded(entered.promise, `${kind} registered provider middleware`);
    const stop = observed(f.client.stop(5000));
    await bounded(f.shutdown, `${kind} shutdown while connection reader remains alive`);
    resume.resolve();
    const outcome = await bounded(pending, `${kind} provider retirement outcome`);
    assert.equal(outcome.status, 'rejected', `${kind} provider retirement must not fulfill with null/default language work`);
    assert.ok(outcome.reason instanceof vscode.CancellationError || [-32096, -32800].includes(outcome.reason.code),
      'the registered provider must preserve cancellation or local inactivity');
    const method = kind === 'semantic' ? 'textDocument/semanticTokens/full' : 'textDocument/formatting';
    assert.equal(f.outbound.some(message => message.method === method), false, 'suspended provider cannot transmit after retirement');
    f.acknowledge.resolve(null);
    assert.equal((await bounded(stop, `${kind} stop completion`)).status, 'fulfilled');
  } finally { resume.resolve(); await f.close(); }
}
async function stoppedOwnedChannel(SessionLanguageClient, sdk, observer) {
  const f = fixture(SessionLanguageClient, sdk);
  let owned;
  try {
    await bounded(f.client.start(), 'owned-channel client startup');
    const output = f.client.outputChannel; // Force the SDK's default-owned resource.
    output.appendLine('consumer owned-channel positive probe');
    owned = observer.channels.find(entry => entry.channel.name === output.name && !entry.disposed);
    assert.ok(owned, 'the real VS Code factory must have created the SDK-owned channel');
    f.closeUnexpectedly();
    await bounded(f.stopped, 'Stopped after unexpected close');
    await bounded(f.closed, 'DoNotRestart close cleanup callback');
    await bounded(f.client.dispose(), 'terminal disposal of stopped client');
    assert.equal(owned.disposed, true, 'disposing a stopped/resource-cleared client must dispose its actual SDK-owned output channel');
  } finally { await f.close(); if (owned && !owned.disposed) owned.channel.dispose(); }
}
async function lateProcessOutput(SessionLanguageClient, sdk, observer) {
  const root = await fs.mkdtemp(path.join(process.env.DAML_CONSUMER_SCRATCH, 'late-output-'));
  const control = path.join(root, 'control');
  const events = path.join(root, 'events.jsonl');
  await fs.writeFile(control, '');
  await fs.writeFile(events, '');
  const stderr = deferred();
  const exited = deferred();
  const spawn = cp.spawn;
  let child;
  cp.spawn = function (...args) {
    const actual = Reflect.apply(spawn, this, args);
    if (args[2]?.env?.DAML_CONSUMER_EVENTS === events) {
      child = actual;
      actual.stderr.on('data', chunk => { if (chunk.toString().includes('DAML consumer late stderr')) stderr.resolve(); });
      actual.once('close', () => exited.resolve());
    }
    return actual;
  };
  const stopping = [];
  const client = new SessionLanguageClient('late-output-check', 'DAML late output check', {
    command: process.execPath, args: [path.join(__dirname, 'consumer-node-probe.cjs')],
    options: { shell: false, env: { ...process.env, ELECTRON_RUN_AS_NODE: '1', DAML_CONSUMER_PACKAGE: packagePath,
      DAML_CONSUMER_CONTROL: control, DAML_CONSUMER_EVENTS: events } },
  }, { documentSelector: [] }, {
    isCurrent: () => true,
    errorHandler: { error: () => ({ action: sdk.ErrorAction.Continue }), closed: () => ({ action: sdk.CloseAction.DoNotRestart, handled: true }) },
    onClosed() {}, onStop(promise) { stopping.push(observed(promise)); },
  });
  try {
    await bounded(client.start(), 'real Node probe startup');
    client.outputChannel.appendLine('consumer late-output positive probe');
    assert.ok(child?.pid, 'stock Node owner must create the actual probe process');
    await bounded(client.dispose(), 'real Node protocol retirement');
    const channelBoundary = observer.channels.length;
    await fs.writeFile(control, 'stderr');
    await bounded(stderr.promise, 'actual retired child stderr event');
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(observer.channels.length, channelBoundary, 'late old-process stderr must not reacquire a real VS Code output channel');
    await fs.writeFile(control, 'exit');
    await bounded(exited.promise, 'actual retired child exit');
    assert.equal(observer.channels.length, channelBoundary, 'late old-process exit must not reacquire UI ownership');
  } finally {
    await client.dispose(100).catch(() => {});
    if (child && child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await bounded(exited.promise, 'probe cleanup'); }
    cp.spawn = spawn;
    await Promise.all(stopping);
    await fs.rm(root, { recursive: true, force: true });
  }
}
async function run() {
  const sdkRoot = path.dirname(dependencyRequire.resolve('vscode-languageclient/package.json'));
  const compiled = process.env.DAML_SESSION_CLIENT_MODULE;
  const observer = observeVscode(filename => filename === compiled || filename?.startsWith(`${sdkRoot}${path.sep}`));
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  try {
    const sdk = dependencyRequire('vscode-languageclient/node');
    const { SessionLanguageClient } = require(compiled);
    const selected = process.env.DAML_CONSUMER_CASE;
    const cases = [
      ['semantic-provider', () => suspendedProvider(SessionLanguageClient, sdk, 'semantic')],
      ['format-provider', () => suspendedProvider(SessionLanguageClient, sdk, 'format')],
      ['owned-output', () => stoppedOwnedChannel(SessionLanguageClient, sdk, observer)],
      ['late-process-output', () => lateProcessOutput(SessionLanguageClient, sdk, observer)],
      ['semantic-inflight', () => submittedSemanticProvider(SessionLanguageClient, sdk)],
    ];
    assert.ok(!selected || cases.some(([name]) => name === selected), 'Unknown SDK consumer case');
    const failures = [];
    for (const [name, check] of cases) {
      if (selected && selected !== name) continue;
      try { await check(); console.log(`PASS SDK consumer: ${name}`); }
      catch (error) { console.error(`FAIL SDK consumer: ${name}`, error); failures.push(new Error(name, { cause: error })); }
    }
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
    if (failures.length) throw new AggregateError(failures, 'Real SDK consumer regressions failed');
  } finally { process.removeListener('unhandledRejection', onUnhandled); observer.restore(); }
}
module.exports = { run };
