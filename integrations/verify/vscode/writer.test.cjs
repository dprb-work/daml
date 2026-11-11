const assert = require('node:assert/strict');
const { test } = require('node:test');
const { PassThrough, Writable } = require('node:stream');
const { createRequire } = require('node:module');
const path = require('node:path');
const frames = require('./lsp-frames.cjs');
const dependencyRequire = createRequire(path.join(__dirname, '../../vscode/package.json'));
const rpc = dependencyRequire('vscode-jsonrpc/node');
const inactive = -32096;
function deferred() {
  let resolve;
  const promise = new Promise(yes => { resolve = yes; });
  return { promise, resolve };
}
function observed(promise) {
  return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
}
async function refused(promise) {
  const outcome = await promise;
  assert.equal(outcome.status, 'rejected', 'retired work cannot report remote success');
  assert.equal(outcome.reason.code, inactive, 'refusal must retain the local inactive-session classification');
}
function fixture(SessionMessageWriter, current = () => true) {
  const messages = [];
  const entered = deferred();
  const release = deferred();
  let hold = false;
  let held = false;
  const decode = frames(message => messages.push(message));
  const output = new Writable({
    write(chunk, encoding, callback) {
      decode(Buffer.from(chunk));
      if (hold && !held) {
        held = true;
        entered.resolve();
        release.promise.then(error => callback(error));
      } else callback();
    },
  });
  const stock = new rpc.StreamMessageWriter(output);
  const writer = new SessionMessageWriter(stock, current);
  return {
    writer, messages, entered: entered.promise,
    hold() { hold = true; },
    release() { release.resolve(); },
    fail(error) { release.resolve(error); },
    close() { release.resolve(); writer.dispose(); output.destroy(); },
  };
}
const request = (id, method, params) => ({ jsonrpc: '2.0', id, method, params });
const notification = (method, params) => ({ jsonrpc: '2.0', method, params });
async function initialized(f) {
  await f.writer.write(request(0, 'initialize', {}));
  await f.writer.write(notification('initialized', {}));
}

async function bootstrap(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  try {
    await refused(observed(f.writer.write(request(1, 'textDocument/semanticTokens/full', {}))));
    await f.writer.write(request(0, 'initialize', {}));
    await refused(observed(f.writer.write(request(2, 'initialize', {}))));
    // Replies in the initialization exchange are necessary bootstrap traffic.
    await f.writer.write({ jsonrpc: '2.0', id: 'server-bootstrap-request', result: null });
    await refused(observed(f.writer.write(notification('textDocument/didOpen', {}))));
    await f.writer.write(notification('initialized', {}));
    await f.writer.write(notification('textDocument/didOpen', {}));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', undefined, 'initialized', 'textDocument/didOpen']);
  } finally { f.close(); }
}

async function retiredBootstrap(SessionMessageWriter) {
  let current = true;
  const f = fixture(SessionMessageWriter, () => current);
  try {
    f.hold();
    const initialize = f.writer.write(request(0, 'initialize', {}));
    await f.entered;
    current = false;
    f.writer.retire();
    f.release();
    await initialize;
    current = true; // A later desired configuration must not reopen this instance.
    await refused(observed(f.writer.write(notification('initialized', {}))));
    await refused(observed(f.writer.write(request(1, 'shutdown'))));
    await refused(observed(f.writer.write(notification('exit'))));
    await refused(observed(f.writer.write({ jsonrpc: '2.0', id: 'late-bootstrap-reply', result: null })));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize']);
  } finally { f.close(); }
}

async function staleInitialization(SessionMessageWriter) {
  let current = true;
  const f = fixture(SessionMessageWriter, () => current);
  try {
    await f.writer.write(request(0, 'initialize', {}));
    current = false;
    await refused(observed(f.writer.write(notification('initialized', {}))));
    await refused(observed(f.writer.write(notification('textDocument/didOpen', {}))));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize']);
  } finally { f.close(); }
}

async function refusedWhileDeliveryPending(promise) {
  const outcome = await Promise.race([
    observed(promise),
    new Promise(resolve => setImmediate(() => resolve({ status: 'pending' }))),
  ]);
  assert.equal(outcome.status, 'rejected',
    'ordinary admission must refuse locally, not queue behind pending initialized delivery');
  assert.equal(outcome.reason.code, inactive);
}

async function initializedDeliveryOpensAdmission(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  try {
    await f.writer.write(request(0, 'initialize', {}));
    f.hold();
    const delivery = observed(f.writer.write(notification('initialized', {})));
    await f.entered;
    await refusedWhileDeliveryPending(f.writer.write(notification('textDocument/didOpen', {})));
    f.release();
    assert.equal((await delivery).status, 'fulfilled');
    await f.writer.write(notification('textDocument/didOpen', {}));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized', 'textDocument/didOpen'],
      'only successful initialized delivery opens ordinary admission; refused work is not replayed');
  } finally { f.close(); }
}

async function retirementDuringInitializedDelivery(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  try {
    await f.writer.write(request(0, 'initialize', {}));
    f.hold();
    const delivery = observed(f.writer.write(notification('initialized', {})));
    await f.entered;
    f.writer.retire();
    await refusedWhileDeliveryPending(f.writer.write(request(1, 'textDocument/semanticTokens/full', {})));
    f.release();
    const completed = await delivery;
    // An already-admitted notification may finish, or report local retirement;
    // neither outcome permits its completion to reopen ordinary admission.
    if (completed.status === 'rejected') assert.equal(completed.reason.code, inactive);
    await refused(observed(f.writer.write(request(2, 'textDocument/semanticTokens/full', {}))));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized']);
  } finally { f.close(); }
}

async function failedInitializedDelivery(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  try {
    await f.writer.write(request(0, 'initialize', {}));
    f.hold();
    const delivery = observed(f.writer.write(notification('initialized', {})));
    await f.entered;
    f.fail(new Error('Deliberate initialized Writable callback failure'));
    assert.equal((await delivery).status, 'rejected', 'bootstrap delivery failure must reach its caller');
    await refused(observed(f.writer.write(notification('textDocument/didOpen', {}))));
    assert.equal(f.messages.some(message => message.method === 'textDocument/didOpen'), false,
      'failed initialized delivery never opens ordinary admission');
  } finally { f.close(); }
}

async function suspendedTraffic(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  const resume = deferred();
  const late = [
    request(2, 'textDocument/semanticTokens/full', { textDocument: { uri: 'file:///probe.daml' } }),
    request(3, 'workspace/executeCommand', { command: 'daml.setContext', arguments: [] }),
    notification('textDocument/didChange', { contentChanges: [{ text: '{}' }] }),
    notification('workspace/didChangeWatchedFiles', { changes: [] }),
    notification('$/setTrace', { value: 'verbose' }),
    notification('$/cancelRequest', { id: 1 }),
    notification('$/progress', { token: 'p', value: {} }),
    { jsonrpc: '2.0', id: 'server-request', result: { success: true } },
    { jsonrpc: '2.0', id: 'server-error', error: { code: -32603, message: 'late error response' } },
  ];
  try {
    await initialized(f);
    const pending = late.map(message => observed((async () => { await resume.promise; return f.writer.write(message); })()));
    f.writer.retire();
    await f.writer.write(request(10, 'shutdown'));
    resume.resolve();
    for (const result of pending) await refused(result);
    await refused(observed(f.writer.write(request(11, 'shutdown'))));
    // ACK ordering remains the real language client's responsibility; this
    // component only admits exit after its one legal shutdown submission.
    await f.writer.write(notification('exit'));
    await refused(observed(f.writer.write(notification('exit'))));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized', 'shutdown', 'exit']);
  } finally { resume.resolve(); f.close(); }
}

async function admittedFifo(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  try {
    await initialized(f);
    f.hold();
    const ordinary = f.writer.write(notification('textDocument/didChange', { contentChanges: [{ text: '{}' }] }));
    await f.entered;
    f.writer.retire();
    const shutdown = f.writer.write(request(4, 'shutdown'));
    f.release();
    await Promise.all([ordinary, shutdown]);
    await f.writer.write(notification('exit'));
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized', 'textDocument/didChange', 'shutdown', 'exit'],
      'already-admitted bytes retain stock writer FIFO across retirement');
  } finally { f.close(); }
}

async function refusedRpcRequest(SessionMessageWriter) {
  const f = fixture(SessionMessageWriter);
  const input = new PassThrough();
  const connection = rpc.createMessageConnection(new rpc.StreamMessageReader(input), f.writer);
  try {
    await initialized(f);
    connection.listen();
    f.writer.retire();
    await refused(observed(connection.sendRequest('textDocument/semanticTokens/full', {})));
    assert.equal(connection.hasPendingResponse(), false);
    assert.deepEqual(f.messages.map(message => message.method), ['initialize', 'initialized'],
      'a real RPC caller must settle locally without transmitting refused work');
  } finally { connection.dispose(); input.destroy(); f.close(); }
}

async function bounded(check, Writer) {
  let timer;
  try {
    return await Promise.race([check(Writer), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`Writer check did not settle: ${check.name}`)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

async function run() {
  const { SessionMessageWriter } = require(process.env.DAML_WRITER_MODULE);
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  try {
    for (const check of [
      bootstrap, retiredBootstrap, staleInitialization, suspendedTraffic, admittedFifo, refusedRpcRequest,
      initializedDeliveryOpensAdmission, retirementDuringInitializedDelivery, failedInitializedDelivery,
    ]) {
      await bounded(check, SessionMessageWriter);
      console.log(`PASS session writer: ${check.name}`);
    }
    // Effectiveness probe: suppress only the real retirement transition. This
    // plausible defective alternative must fail the same observable assertion.
    class WriterWithoutRetirement extends SessionMessageWriter {
      retire() {}
    }
    await assert.rejects(() => bounded(retiredBootstrap, WriterWithoutRetirement), assert.AssertionError);
    console.log('PASS session writer oracle: missing retirement is rejected');
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
  } finally { process.removeListener('unhandledRejection', onUnhandled); }
}
test('connection-scoped session writer admission, settlement and retirement', run);
