const assert = require('node:assert/strict');
const { PassThrough } = require('node:stream');
const { createRequire } = require('node:module');
const path = require('node:path');
const vscode = require('vscode');
const rpc = createRequire(path.join(__dirname, '../../vscode/package.json'))('vscode-jsonrpc/node');
function deferred() { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; }
function observed(promise) { return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason })); }
async function bounded(promise, label) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`No submitted-provider observation: ${label}`)), 5000); })]); }
  finally { clearTimeout(timer); }
}

// Distinct from middleware suspension: this real registered provider has already
// submitted its request, and connection disposal must settle that exact caller.
module.exports = async function submittedSemanticProvider(SessionLanguageClient, sdk) {
  const input = new PassThrough();
  const output = new PassThrough();
  const outbound = [];
  const workEntered = deferred();
  const workResult = deferred();
  const protocolExit = deferred();
  const stopping = [];
  const server = rpc.createMessageConnection(new rpc.StreamMessageReader(output), new rpc.StreamMessageWriter(input));
  server.onRequest('initialize', () => ({ capabilities: {
    semanticTokensProvider: { legend: { tokenTypes: ['string'], tokenModifiers: [] }, full: true },
  } }));
  server.onRequest('textDocument/semanticTokens/full', params => {
    workEntered.resolve(params);
    return workResult.promise;
  });
  server.onRequest('shutdown', () => null);
  server.onNotification('exit', () => protocolExit.resolve());
  server.listen();
  const writer = new rpc.StreamMessageWriter(output);
  const client = new SessionLanguageClient('submitted-provider-check', 'DAML submitted provider check', () => Promise.resolve({
    reader: new rpc.StreamMessageReader(input), writer: {
      onError: writer.onError, onClose: writer.onClose,
      write(message) { outbound.push(message); return writer.write(message); },
      end() { writer.end(); }, dispose() { writer.dispose(); },
    },
  }), { documentSelector: [{ scheme: 'untitled', language: 'plaintext' }] }, {
    isCurrent: () => true,
    errorHandler: { error: () => ({ action: sdk.ErrorAction.Continue }), closed: () => ({ action: sdk.CloseAction.DoNotRestart, handled: true }) },
    onClosed(decision) { assert.equal(decision.action, sdk.CloseAction.DoNotRestart); },
    onStop(promise) { stopping.push(observed(promise)); },
  });
  try {
    await bounded(client.start(), 'actual SessionLanguageClient startup');
    const document = await vscode.workspace.openTextDocument({ language: 'plaintext', content: '{}' });
    const provider = client.getFeature(sdk.SemanticTokensRegistrationType.type.method).getProvider(document).full;
    const pending = observed(provider.provideDocumentSemanticTokens(document, rpc.CancellationToken.None));
    const submitted = await bounded(workEntered.promise, 'semantic request received by real protocol peer');
    assert.equal(submitted.textDocument.uri, document.uri.toString());
    assert.equal(outbound.filter(message => message.method === 'textDocument/semanticTokens/full').length, 1,
      'the provider request must already be admitted on this connection');
    await bounded(client.stop(5000), 'shutdown/exit and connection disposal with unanswered semantic work');
    await bounded(protocolExit.promise, 'real peer received terminal exit');
    const outcome = await bounded(pending, 'submitted provider after connection disposal');
    assert.equal(outcome.status, 'rejected', 'disposal of submitted semantic work must not fulfill with null/default language work');
    assert.ok(outcome.reason instanceof vscode.CancellationError || [-32096, -32800].includes(outcome.reason.code),
      'the public provider must preserve cancellation or inactive-session classification');
    const shutdown = outbound.findIndex(message => message.method === 'shutdown');
    assert.deepEqual(outbound.slice(shutdown + 1).map(message => message.method), ['exit']);
  } finally {
    // The peer owns its unresolved handler until deliberately released here;
    // no forever-pending fixture promise is abandoned after the observation.
    workResult.resolve({ data: [] });
    await new Promise(resolve => setImmediate(resolve));
    await client.dispose(100).catch(() => {});
    server.dispose();
    input.destroy();
    output.destroy();
    await Promise.all(stopping);
  }
};
