const assert = require('node:assert/strict');
const { PassThrough } = require('node:stream');
const { createRequire } = require('node:module');
const path = require('node:path');
const cp = require('node:child_process');
const fs = require('node:fs/promises');
const { tmpdir } = require('node:os');
const dependencyRequire = createRequire(path.join(__dirname, '../../vscode/package.json'));
const rpc = dependencyRequire('vscode-jsonrpc/node');
const { BaseLanguageClient, LanguageClient, State, CloseAction, ErrorAction } = dependencyRequire('vscode-languageclient');

function deferred() {
  let resolve;
  const promise = new Promise(yes => { resolve = yes; });
  return { promise, resolve };
}
function observed(promise) {
  return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
}
async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), 3000);
    })]);
  } finally { clearTimeout(timer); }
}

// Only the supported transport seam is supplied. Startup, initialization,
// request tracking, shutdown and disposal are the actual 9.0.1 implementation,
// running with real VS Code and real JSONRPC, not a fake editor API.
function fixture({ acquisition, initialize, shutdown, disposeFailure, useDefaultErrorHandler = false } = {}) {
  const clientToServer = new PassThrough();
  const serverToClient = new PassThrough();
  const messages = [];
  const transportDisposed = deferred();
  const initializing = deferred();
  const unanswered = deferred();
  const acquiring = deferred();
  const server = rpc.createMessageConnection(
    new rpc.StreamMessageReader(clientToServer), new rpc.StreamMessageWriter(serverToClient),
  );
  server.onRequest('initialize', params => {
    initializing.resolve(params);
    return initialize ? initialize(params) : { capabilities: {} };
  });
  server.onRequest('shutdown', () => shutdown ? shutdown() : null);
  server.onRequest('probe/unanswered', () => { unanswered.resolve(); return new Promise(() => {}); });
  server.onNotification('exit', () => {});
  server.listen();
  class Client extends BaseLanguageClient {
    constructor() {
      super('daml-lifecycle-check', 'DAML lifecycle dependency check', {
        documentSelector: [],
        errorHandler: useDefaultErrorHandler ? undefined : { error: () => ({ action: ErrorAction.Continue }), closed: () => ({ action: CloseAction.DoNotRestart }) },
      });
    }
    async createMessageTransports() {
      acquiring.resolve();
      if (acquisition) await acquisition;
      const reader = new rpc.StreamMessageReader(serverToClient);
      const writer = new rpc.StreamMessageWriter(clientToServer);
      return {
        reader,
        writer: {
          onError: writer.onError,
          onClose: writer.onClose,
          write(message) { messages.push(message); return writer.write(message); },
          end() { writer.end(); },
          dispose() {
            writer.dispose();
            transportDisposed.resolve();
            if (disposeFailure !== undefined) throw disposeFailure;
          },
        },
      };
    }
    // Do not display modal error UI for errors deliberately induced by a test.
    error() {}
  }
  const client = new Client();
  return {
    client, messages, initializing: initializing.promise, transportDisposed: transportDisposed.promise,
    unanswered: unanswered.promise, acquiring: acquiring.promise,
    // The real stream listener forwards this through reader.onError and the
    // JSONRPC connection; no client lifecycle handler is invoked by the fixture.
    emitReaderError(error) { serverToClient.emit('error', error); },
    async close() {
      await client.dispose(50).catch(() => {});
      server.dispose();
      clientToServer.destroy();
      serverToClient.destroy();
    },
  };
}

async function failedStopDisposesConnection(timeout) {
  const failure = new rpc.ResponseError(-32603, 'Deliberate shutdown failure');
  const f = fixture({ shutdown: () => timeout ? new Promise(() => {}) : Promise.reject(failure) });
  try {
    await bounded(f.client.start(), 'client startup');
    const unanswered = observed(f.client.sendRequest('probe/unanswered'));
    await bounded(f.unanswered, 'unanswered request reaching the peer');
    const stop = await bounded(observed(f.client.stop(timeout ? 25 : 1000)), 'failed stop');
    assert.equal(stop.status, 'rejected', 'retirement failure must not be reported as successful stop');
    const pending = await bounded(unanswered, 'caller owned by failed stop');
    assert.equal(pending.status, 'rejected', 'failure must dispose the captured connection and reject pending callers');
    await bounded(f.transportDisposed, 'failed stop transport disposal');
    assert.equal(f.messages.some(message => message.method === 'exit'), false, 'failed shutdown cannot authorize exit');
  } finally { await f.close(); }
}

async function shutdownWithDisposeFailure(failedShutdown) {
  const cleanupFailure = new Error('Deliberate writer disposal failure after underlying cleanup');
  const shutdownFailure = new rpc.ResponseError(-32603, 'Deliberate shutdown failure');
  const f = fixture({
    disposeFailure: cleanupFailure,
    shutdown: () => failedShutdown ? Promise.reject(shutdownFailure) : null,
  });
  try {
    await bounded(f.client.start(), 'client startup before disposal failure');
    const unanswered = observed(f.client.sendRequest('probe/unanswered'));
    await bounded(f.unanswered, 'pending request reaching peer before disposal failure');
    const stop = await bounded(observed(f.client.stop(1000)), 'stop with throwing writer disposal');
    assert.equal(stop.status, 'rejected', 'cleanup failure cannot establish successful retirement');
    if (failedShutdown) {
      assert.equal(stop.reason.code, -32603, 'cleanup failure must not replace the original remote shutdown category');
      assert.equal(stop.reason.message, shutdownFailure.message);
    } else {
      assert.equal(stop.reason, cleanupFailure, 'successful shutdown must surface the actual cleanup failure');
    }
    assert.equal((await bounded(unanswered, 'caller owned by throwing disposal')).status, 'rejected',
      'pending callers must settle despite a resource-disposal exception');
    await bounded(f.transportDisposed, 'real underlying writer disposal');
    assert.equal(f.messages.some(message => message.method === 'exit'), !failedShutdown,
      'only successful shutdown acknowledgement authorizes exit');
  } finally { await f.close(); }
}

async function cancelInitialization() {
  const lateResult = deferred();
  const f = fixture({ initialize: () => lateResult.promise });
  const states = [];
  const subscription = f.client.onDidChangeState(event => states.push(event.newState));
  try {
    const start = observed(f.client.start());
    await bounded(f.initializing, 'initialize request');
    await bounded(f.client.dispose(100), 'Starting disposal');
    assert.equal((await bounded(start, 'cancelled startup')).status, 'rejected');
    await bounded(f.transportDisposed, 'cancelled startup transport disposal');
    lateResult.resolve({ capabilities: { textDocumentSync: 1 } });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(states.includes(State.Running), false, 'late initialize completion cannot publish Running');
    assert.deepEqual(f.messages.map(message => message.method), ['initialize'], 'uninitialized retirement cannot send shutdown, exit or initialized');
  } finally {
    lateResult.resolve({ capabilities: {} });
    subscription.dispose();
    await f.close();
  }
}

async function startupReaderFailure(shutdownOnError) {
  const lateResult = deferred();
  const cause = new Error('Deliberate underlying startup transport failure');
  const failure = Object.assign(new Error('Deliberate reader error during initialization', { cause }), { code: 'EPIPE' });
  const f = fixture({
    initialize: () => lateResult.promise,
    useDefaultErrorHandler: shutdownOnError,
  });
  const states = [];
  const subscription = f.client.onDidChangeState(event => states.push(event.newState));
  try {
    const start = observed(f.client.start());
    await bounded(f.initializing, 'initialize request before reader error');
    assert.equal(f.client.state, State.Starting);
    f.emitReaderError(failure);
    if (shutdownOnError) {
      // Reader errors have no writer retry count. The actual default handler
      // selects Shutdown; cancellation must not replace the originating error.
      const result = await bounded(start, 'startup rejected by reader error');
      assert.equal(result.status, 'rejected');
      assert.equal(result.reason, failure, 'startup must reject with the original transport error, not generic cancellation');
      assert.equal(result.reason.code, 'EPIPE');
      assert.equal(result.reason.cause, cause);
      await bounded(f.transportDisposed, 'reader-error startup connection disposal');
      lateResult.resolve({ capabilities: {} });
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(states.includes(State.Running), false, 'late initialization cannot activate a failed transport');
      assert.deepEqual(f.messages.map(message => message.method), ['initialize'],
        'reader-error retirement before initialization cannot send shutdown, exit or initialized');
    } else {
      // Drain the real asynchronous policy decision before releasing initialize.
      // Continue must not save a latent failure that poisons successful startup.
      await new Promise(resolve => setImmediate(resolve));
      lateResult.resolve({ capabilities: {} });
      assert.equal((await bounded(start, 'startup after continued reader error')).status, 'fulfilled');
      assert.equal(f.client.state, State.Running);
      assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized']);
      await bounded(f.client.stop(1000), 'normal shutdown after continued reader error');
      await bounded(f.transportDisposed, 'continued reader-error connection cleanup');
      assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized', 'shutdown', 'exit']);
    }
  } finally {
    lateResult.resolve({ capabilities: {} });
    subscription.dispose();
    await f.close();
  }
}

async function cancelTransportAcquisition() {
  const acquired = deferred();
  const f = fixture({ acquisition: acquired.promise });
  const states = [];
  const subscription = f.client.onDidChangeState(event => states.push(event.newState));
  try {
    const start = observed(f.client.start());
    await bounded(f.acquiring, 'transport acquisition entry');
    assert.equal(f.client.state, State.Starting);
    await bounded(f.client.dispose(100), 'Starting disposal before transport acquisition');
    assert.equal((await bounded(start, 'cancelled acquisition startup')).status, 'rejected');
    acquired.resolve();
    await bounded(f.transportDisposed, 'late acquired transport disposal');
    assert.deepEqual(f.messages, [], 'late acquired resources must be disposed without initialization');
    assert.equal(states.includes(State.Running), false);
  } finally {
    acquired.resolve();
    subscription.dispose();
    await f.close();
  }
}

async function cancelNodeWorkingDirectoryLookup() {
  const scratch = await fs.mkdtemp(path.join(tmpdir(), 'daml-node-cwd-cancel-'));
  const marker = path.join(scratch, 'native-process-started');
  const wire = path.join(scratch, 'native-stdin');
  const cwdEntered = deferred();
  const cwd = deferred();
  const acquisitionSettled = deferred();
  const children = [];
  const states = [];
  const spawn = cp.spawn;
  // Version-sensitive fixture seam in pinned Node LanguageClient 9.0.1:
  // _getServerWorkingDir owns the asynchronous filesystem lookup before spawn.
  // Only that lookup is delayed; launch, cancellation and cleanup remain real.
  class NodeClient extends LanguageClient {
    _getServerWorkingDir() {
      cwdEntered.resolve();
      return cwd.promise;
    }
    async createMessageTransports(...args) {
      try { return await super.createMessageTransports(...args); }
      finally { acquisitionSettled.resolve(); }
    }
    error() {}
  }
  const nativeProbe = [
    "const fs = require('node:fs');",
    "fs.writeFileSync(process.env.DAML_CWD_PROBE, 'spawned');",
    "process.stdin.on('data', chunk => fs.appendFileSync(process.env.DAML_CWD_WIRE, chunk));",
    'process.stdin.resume();',
  ].join('\n');
  const client = new NodeClient('daml-node-cwd-check', 'DAML Node cwd cancellation check', {
    command: process.execPath, args: ['-e', nativeProbe],
    options: {
      shell: false,
      env: { ...process.env, ELECTRON_RUN_AS_NODE: '1', DAML_CWD_PROBE: marker, DAML_CWD_WIRE: wire },
    },
  }, {
    documentSelector: [],
    errorHandler: { error: () => ({ action: ErrorAction.Continue }), closed: () => ({ action: CloseAction.DoNotRestart }) },
  });
  const subscription = client.onDidChangeState(event => states.push(event.newState));
  // Forward to the actual OS process owner. Any captured ChildProcess/PID is
  // a forbidden native-resource effect, not a mocked forwarding call count.
  cp.spawn = function (...args) {
    const child = Reflect.apply(spawn, this, args);
    if (args[2]?.env?.DAML_CWD_PROBE === marker) children.push(child);
    return child;
  };
  try {
    const start = observed(client.start());
    await bounded(cwdEntered.promise, 'Node working-directory lookup entry');
    assert.equal(client.state, State.Starting);
    await bounded(client.dispose(100), 'Node Starting disposal while cwd is unresolved');
    assert.equal((await bounded(start, 'Node startup cancelled before cwd resolution')).status, 'rejected',
      'startup cancellation cannot await filesystem lookup completion');
    cwd.resolve(scratch);
    await bounded(acquisitionSettled.promise, 'retired Node transport acquisition');
    // Let the released cwd continuation and its promise reactions run. No
    // timing sleep creates the interleaving: the lookup was explicitly held.
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(children.map(child => ({ pid: child.pid ?? null, executable: child.spawnfile })), [],
      'retired cwd continuation must not create an actual native ChildProcess');
    for (const file of [marker, wire]) {
      await assert.rejects(fs.access(file), { code: 'ENOENT' },
        'retired startup cannot execute the native probe or emit late bootstrap bytes');
    }
    assert.equal(states.includes(State.Running), false);
  } finally {
    cwd.resolve(scratch);
    try {
      await bounded(acquisitionSettled.promise, 'Node acquisition cleanup').catch(() => {});
      await client.dispose(50).catch(() => {});
      for (const child of children) {
        if (child.exitCode !== null || child.signalCode !== null) continue;
        const closed = new Promise(resolve => child.once('close', resolve));
        child.kill('SIGKILL');
        await bounded(closed, 'test-owned native process cleanup');
      }
    } finally {
      subscription.dispose();
      cp.spawn = spawn;
      await fs.rm(scratch, { recursive: true, force: true });
    }
  }
}

async function run() {
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  try {
    const selected = process.env.DAML_CLIENT_DEPENDENCY_CASE;
    assert.ok(selected === undefined || ['node-cwd', 'dispose-failure', 'startup-error'].includes(selected), 'Unknown dependency host case selection');
    const failures = [];
    for (const [name, check, group] of [
      ['failed shutdown disposes pending connection', () => failedStopDisposesConnection(false)],
      ['shutdown timeout disposes pending connection', () => failedStopDisposesConnection(true)],
      ['Starting cancellation during initialization', cancelInitialization],
      ['Starting cancellation during acquisition', cancelTransportAcquisition],
      ['Node Starting cancellation before spawn during cwd lookup', cancelNodeWorkingDirectoryLookup],
      ['writer dispose failure after successful shutdown', () => shutdownWithDisposeFailure(false), 'dispose-failure'],
      ['writer dispose failure preserves original shutdown error', () => shutdownWithDisposeFailure(true), 'dispose-failure'],
      ['Starting reader error preserves original failure under default Shutdown policy', () => startupReaderFailure(true), 'startup-error'],
      ['Starting reader error with Continue allows successful initialization', () => startupReaderFailure(false), 'startup-error'],
    ]) {
      if (selected === 'node-cwd' && check !== cancelNodeWorkingDirectoryLookup) continue;
      if (selected === 'dispose-failure' && group !== 'dispose-failure') continue;
      if (selected === 'startup-error' && group !== 'startup-error') continue;
      try {
        await check();
        console.log(`PASS dependency lifecycle: ${name}`);
      } catch (error) {
        console.error(`FAIL dependency lifecycle: ${name}`, error);
        failures.push(new Error(name, { cause: error }));
      }
    }
    if (failures.length > 0) throw new AggregateError(failures, 'Dependency lifecycle checks failed');
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, [], 'dependency lifecycle failures must not escape settled callers');
  } finally {
    process.removeListener('unhandledRejection', onUnhandled);
  }
}
module.exports = { run };
