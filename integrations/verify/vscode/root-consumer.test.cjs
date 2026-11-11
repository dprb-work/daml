const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const vscode = require('vscode');
const observeVscode = require('./vscode-observer.cjs');
function observed(promise) { return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason })); }
async function bounded(promise, label) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Root consumer did not settle: ${label}`)), 10_000); })]); }
  finally { clearTimeout(timer); }
}
async function records() { return (await fs.readFile(process.env.DAML_LSP_STATUS_FILE, 'utf8')).split('\n').slice(0, -1).filter(Boolean).map(line => JSON.parse(line)); }
async function waitFor(predicate, label) {
  const deadline = Date.now() + 10_000;
  for (;;) {
    const value = await predicate();
    if (value) return value;
    assert.ok(Date.now() < deadline, `Missing observable root lifecycle effect: ${label}`);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
async function openProbe() {
  const uri = vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri, 'probe.daml');
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);
  return document;
}
const syntax = document => vscode.commands.executeCommand('definitivelyAssertedDaml.useSyntax', { uri: document.uri.toString() });
const configure = (document, bundle) => vscode.commands.executeCommand('definitivelyAssertedDaml.useConfiguration', {
  uri: document.uri.toString(), bundleRoot: bundle.toString(), expected: 'self.Config',
});
async function replacementInitialized() {
  return waitFor(async () => {
    const events = await records();
    const replacement = events.find(event => event.event === 'wrapper-start' && event.executable === process.env.DAML_ROOT_REPLACEMENT);
    return replacement && events.find(event => event.session === replacement.session && event.event === 'outbound' && event.message.method === 'initialized');
  }, 'replacement initialized after configuration callbacks');
}
async function configurationCompletion(extension, observer) {
  await bounded(extension.activate(), 'initial installed activation');
  const document = await openProbe();
  assert.ok((await bounded(syntax(document), 'initial language operation')).generation > 0);
  const whitespace = `  ${process.env.DAML_ROOT_ORIGINAL}  `;
  observer.beginConfigurationBatch();
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update('serverPath', whitespace, vscode.ConfigurationTarget.Global);
  await waitFor(() => observer.pendingEvents.some(entry => entry.snapshot.get('serverPath') === whitespace), 'genuine whitespace configuration event snapshot');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update('serverPath', process.env.DAML_ROOT_REPLACEMENT, vscode.ConfigurationTarget.Global);
  await waitFor(() => observer.pendingEvents.some(entry => entry.snapshot.get('serverPath') === process.env.DAML_ROOT_REPLACEMENT), 'genuine replacement configuration event snapshot');
  // WorkspaceConfiguration objects are real API snapshots, not fabricated get()
  // values. Verify their historical values survived the second actual update.
  assert.deepEqual([...new Set(observer.pendingEvents.map(entry => entry.snapshot.get('serverPath')))],
    [whitespace, process.env.DAML_ROOT_REPLACEMENT]);
  assert.equal(vscode.workspace.getConfiguration('definitivelyAssertedDaml').get('serverPath'), process.env.DAML_ROOT_REPLACEMENT);
  observer.flushConfigurationBatch();
  assert.equal(observer.reads[0], whitespace);
  assert.ok(observer.reads.indexOf(process.env.DAML_ROOT_REPLACEMENT) > 0,
    'the actual root callbacks must read both genuine snapshots in one synchronous batch');
  const replacement = await replacementInitialized();
  const result = await bounded(syntax(document), 'language work after batched configuration completion');
  assert.ok(result.generation > 0);
  const events = await records();
  assert.equal(events.filter(event => event.event === 'wrapper-start').length, 2);
  assert.ok(events.some(event => event.session === replacement.session && event.event === 'outbound' && event.message.method === 'workspace/executeCommand'));
}
function targets(record, file) {
  return record.pattern?.baseUri?.fsPath === path.dirname(file) && record.pattern.pattern === path.basename(file);
}
async function staleContext(extension, observer) {
  await bounded(extension.activate(), 'initial installed activation for stale context');
  const document = await openProbe();
  const bundle = vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri, 'bundle');
  const definition = vscode.Uri.joinPath(bundle, 'Config.daml');
  const pending = observed(configure(document, bundle));
  const held = await waitFor(async () => (await records()).find(event => event.event === 'context-response-held'), 'real dependency-producing response held');
  assert.ok(held.message.result?.generation > 0, 'the held native context must have succeeded');
  const dependencies = await waitFor(async () => (await records()).find(event => event.event === 'dependencies-held'
    && event.message.params.generation === held.message.result.generation), 'correlated native dependency snapshot held');
  assert.equal(dependencies.message.params.uri, document.uri.toString());
  assert.equal(dependencies.message.params.version, document.version);
  assert.ok(dependencies.message.params.files.includes(definition.toString()), 'the genuine context must be capable of installing the selected dependency watcher');
  const queued = observed(configure(document, bundle));
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update('serverPath', process.env.DAML_ROOT_REPLACEMENT, vscode.ConfigurationTarget.Global);
  await waitFor(async () => (await records()).some(event => event.event === 'shutdown-held'), 'retiring old connection held before shutdown acknowledgment');
  assert.equal((await bounded(pending, 'old unresolved context retirement')).status, 'rejected');
  assert.equal((await bounded(queued, 'old queued context retirement')).status, 'rejected');
  assert.equal(observer.watchers.filter(entry => targets(entry, definition.fsPath)).length, 0);
  await fs.writeFile(process.env.DAML_ROOT_CONTROL, 'release-context');
  await waitFor(() => observer.logs.some(text => text.includes(process.env.DAML_ROOT_RELEASE_BEACON)), 'real old-reader log beacon after stale frames');
  // The beacon establishes reader FIFO; this macrotask additionally drains all
  // response -> sendRequest -> context continuation microtasks before asserting.
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(observer.watchers.filter(entry => targets(entry, definition.fsPath)).length, 0,
    'stale successful response/dependencies cannot install a real watcher after retirement');
  const beforeReplacement = await records();
  assert.equal(beforeReplacement.filter(event => event.session === held.session && event.event === 'outbound'
    && event.message.method === 'workspace/executeCommand').length, 1, 'queued context must never escape the retired session');
  assert.equal(beforeReplacement.filter(event => event.event === 'wrapper-start').length, 1, 'the shutdown barrier must keep retirement and stale release on the old reader');
  await fs.writeFile(process.env.DAML_ROOT_CONTROL, 'release-shutdown');
  const replacement = await replacementInitialized();
  const beforeCurrentContext = await records();
  assert.equal(beforeCurrentContext.some(event => event.session === replacement.session && event.event === 'outbound'
    && event.message.method === 'workspace/executeCommand'), false, 'an old queued operation cannot migrate to replacement');
  assert.ok((await bounded(configure(document, bundle), 'genuine current-session dependency context')).generation > 0);
  const watcher = await waitFor(() => observer.watchers.find(entry => targets(entry, definition.fsPath) && !entry.disposed), 'positive real current-session watcher ownership');
  let subscription;
  const changed = new Promise(resolve => {
    subscription = vscode.languages.onDidChangeDiagnostics(event => {
      if (event.uris.some(uri => uri.toString() === document.uri.toString()) && vscode.languages.getDiagnostics(document.uri).length > 0) resolve();
    });
  });
  try {
    await fs.writeFile(definition.fsPath, 'closed !Object { endpoint: !Int }\n');
    await bounded(changed, 'current watcher functional schema invalidation');
  } finally { subscription.dispose(); }
  const afterChange = await records();
  assert.ok(afterChange.some(event => event.session === replacement.session && event.event === 'outbound'
    && event.message.method === 'workspace/didChangeWatchedFiles'
    && event.message.params.changes.some(change => change.uri === definition.toString())),
  'the real observer must see a functional current watcher, not a permanently empty fixture');
  const installed = require(path.resolve(extension.extensionPath, extension.packageJSON.main));
  await bounded(installed.deactivate(), 'current session deactivation');
  assert.equal(watcher.disposed, true, 'the positive current watcher must follow terminal cleanup');
  const oldMessages = (await records()).filter(event => event.session === held.session && event.event === 'outbound').map(event => event.message);
  const shutdown = oldMessages.findIndex(message => message.method === 'shutdown');
  assert.deepEqual(oldMessages.slice(shutdown + 1).map(message => message.method), ['exit']);
}
function failureReceipt(error, observer, extension, unhandled) {
  // Only observed channel output and allowlisted error fields: never dump the
  // process environment, editor documents, or arbitrary error payload objects.
  const roots = [
    [extension.extensionPath, '<installed-extension>'],
    ...vscode.workspace.workspaceFolders.map(folder => [folder.uri.fsPath, '<workspace>']),
    [path.dirname(process.env.DAML_ROOT_ORIGINAL), '<scenario>'],
    [process.env.HOME, '<home>'],
  ].filter(([root]) => root).sort((left, right) => right[0].length - left[0].length);
  function text(value, limit = 2048) {
    let result = String(value)
      .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
      .replace(/[\x00-\x08\x0b-\x1f\x7f]/g, '')
      .replace(/\b(Bearer|Basic)\s+\S+/gi, '$1 <redacted>')
      .replace(/((?:"|')?(?:password|secret|token|api[_-]?key|authorization)(?:"|')?\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;]+)/gi, '$1<redacted>');
    for (const [root, replacement] of roots) result = result.split(root).join(replacement);
    return result.length > limit ? `${result.slice(0, limit)}<truncated>` : result;
  }
  function errorDetails(reason, depth = 0) {
    if (reason === null || typeof reason !== 'object') return text(reason);
    const details = {};
    for (const key of ['name', 'message', 'stack', 'code', 'errno', 'syscall']) {
      if (['string', 'number'].includes(typeof reason[key])) details[key] = text(reason[key], key === 'stack' ? 4096 : 2048);
    }
    if (depth < 2 && reason.cause !== undefined) details.cause = errorDetails(reason.cause, depth + 1);
    if (depth < 2 && Array.isArray(reason.errors)) details.errors = reason.errors.slice(0, 4).map(item => errorDetails(item, depth + 1));
    return details;
  }
  const logStart = Math.max(0, observer.logs.length - 32);
  return {
    event: 'root-consumer-observed-failure',
    scenario: text(process.env.DAML_ROOT_CONSUMER_CASE),
    error: errorDetails(error),
    unhandledCount: unhandled.length,
    unhandled: unhandled.slice(-4).map(reason => errorDetails(reason)),
    channelCount: observer.channels.length,
    channels: observer.channels.slice(-8).map(record => ({
      name: text(record.name), disposed: record.disposed,
      operationCount: record.operationCount, operations: record.operations,
    })),
    logCount: observer.logs.length,
    logs: observer.logs.slice(logStart).map((value, offset) => ({ index: logStart + offset, text: text(value) })),
  };
}
async function run() {
  assert.equal(vscode.workspace.isTrusted, true);
  const extension = vscode.extensions.getExtension('dprb-work.definitively-asserted-daml');
  assert.ok(extension);
  const entry = path.resolve(extension.extensionPath, extension.packageJSON.main);
  const observer = observeVscode(filename => filename === entry);
  const unhandled = [];
  const onUnhandled = reason => unhandled.push(reason);
  process.on('unhandledRejection', onUnhandled);
  try {
    if (process.env.DAML_ROOT_CONSUMER_CASE === 'configuration-completion') await configurationCompletion(extension, observer);
    else if (process.env.DAML_ROOT_CONSUMER_CASE === 'stale-context') await staleContext(extension, observer);
    else throw new Error('Unknown root consumer scenario');
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
  } catch (error) {
    try { console.error(JSON.stringify(failureReceipt(error, observer, extension, unhandled), null, 2)); }
    catch { console.error('Could not collect root consumer observed failure receipt.'); }
    throw error;
  } finally {
    await fs.writeFile(process.env.DAML_ROOT_CONTROL, 'release-shutdown');
    if (require.cache[entry]) await bounded(require(entry).deactivate(), 'root consumer cleanup').catch(() => {});
    observer.restore();
    process.removeListener('unhandledRejection', onUnhandled);
  }
}
module.exports = { run };
