const assert = require('node:assert/strict');
const { PassThrough } = require('node:stream');
const { test } = require('node:test');
const { createRequire } = require('node:module');
const path = require('node:path');
const dependencyRequire = createRequire(path.join(__dirname, '../../vscode/package.json'));
const rpc = dependencyRequire('vscode-jsonrpc/node');

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function observed(promise) {
  return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
}
async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), 2000);
    })]);
  } finally {
    clearTimeout(timer);
  }
}
function fixture(write) {
  const input = new PassThrough();
  const output = new PassThrough();
  const underlying = new rpc.StreamMessageWriter(output);
  const writer = {
    onError: underlying.onError,
    onClose: underlying.onClose,
    write,
    end: () => underlying.end(),
    dispose: () => underlying.dispose(),
  };
  const connection = rpc.createMessageConnection(new rpc.StreamMessageReader(input), writer);
  connection.listen();
  return {
    connection,
    close() { connection.dispose(); input.destroy(); output.destroy(); },
  };
}
function trackedToken() {
  let listeners = 0;
  let disposals = 0;
  return {
    token: {
      isCancellationRequested: false,
      onCancellationRequested() {
        listeners += 1;
        let active = true;
        return { dispose() { disposals += 1; if (active) { active = false; listeners -= 1; } } };
      },
    },
    listeners: () => listeners,
    disposals: () => disposals,
  };
}

test('failed asynchronous writer rejects once, releases cancellation, and emits no unhandled rejection', async () => {
  const write = deferred();
  const entered = deferred();
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  const { connection, close } = fixture(() => { entered.resolve(); return write.promise; });
  const cancellation = trackedToken();
  try {
    const result = observed(connection.sendRequest('probe/refused', {}, cancellation.token));
    await entered.promise;
    assert.equal(cancellation.listeners(), 1);
    write.reject(new Error('Deliberate writer refusal'));
    const outcome = await bounded(result, 'failed write request');
    assert.equal(outcome.status, 'rejected');
    assert.ok(outcome.reason instanceof Error);
    assert.equal(connection.hasPendingResponse(), false, 'failed writes must remove request tracking');
    assert.equal(cancellation.listeners(), 0, 'failed writes must release cancellation subscriptions');
    assert.equal(cancellation.disposals(), 1, 'a caller has exactly one terminal cleanup');
    // The ignored async Promise executor in JSONRPC 8.2.0 rejects on the next turn.
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
  } finally {
    close();
    process.removeListener('unhandledRejection', onUnhandled);
  }
});

test('a request is tracked before an asynchronous writer completes and disposal settles it', async () => {
  const write = deferred();
  const entered = deferred();
  const { connection, close } = fixture(() => { entered.resolve(); return write.promise; });
  const cancellation = trackedToken();
  try {
    const result = observed(connection.sendRequest('probe/delayed', {}, cancellation.token));
    await entered.promise;
    assert.equal(connection.hasPendingResponse(), true, 'registration cannot wait for write completion');
    connection.dispose();
    const outcome = await bounded(result, 'request disposed during its write');
    assert.equal(outcome.status, 'rejected');
    assert.equal(cancellation.listeners(), 0);
    write.resolve();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(connection.hasPendingResponse(), false, 'late writer completion cannot resurrect a disposed request');
    assert.equal(cancellation.disposals(), 1, 'late writer completion cannot repeat terminal cleanup');
  } finally {
    write.resolve();
    close();
  }
});

test('disposal wins over a later failed write without a second cleanup or unhandled rejection', async () => {
  const write = deferred();
  const entered = deferred();
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  const { connection, close } = fixture(() => { entered.resolve(); return write.promise; });
  const cancellation = trackedToken();
  try {
    const result = observed(connection.sendRequest('probe/disposed-write', {}, cancellation.token));
    await entered.promise;
    connection.dispose();
    assert.equal((await bounded(result, 'disposed failed-write request')).status, 'rejected');
    write.reject(new Error('Writer failed after disposal'));
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(cancellation.disposals(), 1);
    assert.equal(cancellation.listeners(), 0);
    assert.equal(connection.hasPendingResponse(), false);
    assert.deepEqual(unhandled, []);
  } finally {
    write.resolve();
    close();
    process.removeListener('unhandledRejection', onUnhandled);
  }
});
