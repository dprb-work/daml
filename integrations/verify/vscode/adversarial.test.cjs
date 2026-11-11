const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { performance } = require('node:perf_hooks');
const vscode = require('vscode');
const extensionId = 'dprb-work.definitively-asserted-daml';

async function events() {
  const text = await fs.readFile(process.env.DAML_LSP_STATUS_FILE, 'utf8');
  return text.split('\n').slice(0, -1).filter(Boolean).map(line => JSON.parse(line));
}
async function waitFor(predicate, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = predicate(await events());
    if (value) return value;
    if (Date.now() >= deadline) throw new Error(`No observation: ${label}`);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
async function bounded(promise, label, timeoutMs) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), timeoutMs);
    })]);
  } finally { clearTimeout(timer); }
}
function observed(promise) {
  return promise.then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
}
function alive(pid) {
  try { process.kill(pid, 0); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}
async function waitForOwnedProcessCleanup() {
  const pids = (await events()).filter(event => event.event === 'wrapper-start' || event.event === 'child-spawn').map(event => event.pid);
  const deadline = Date.now() + 10_000;
  while (pids.some(alive)) {
    assert.ok(Date.now() < deadline, 'cancelled startup must retain bounded wrapper/native-process cleanup ownership');
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
async function deactivate(extension) {
  const installed = require(path.join(extension.extensionPath, extension.packageJSON.main));
  await bounded(installed.deactivate(), 'public deactivation', 10_000);
}

async function startupTimeout(extension) {
  const began = performance.now();
  const activation = observed(extension.activate());
  await waitFor(items => items.find(event => event.event === 'initialize-response-withheld'), 'withheld real initialize response');
  const outcome = await bounded(activation, '30-second startup deadline', 40_000);
  assert.equal(outcome.status, 'rejected', 'startup deadline must report failure, not successful activation');
  assert.ok(performance.now() - began >= 30_000, 'the accepted 30-second startup policy must not be silently shortened');
  assert.ok(performance.now() - began < 40_000, 'external watchdog expired while observing the 30-second policy');
  assert.match(String(outcome.reason), /time(?:d)?\s*out|deadline/i, 'report the timeout, not an unrelated failure');
  await waitForOwnedProcessCleanup();
  await deactivate(extension);
  const captured = await events();
  assert.equal(captured.filter(event => event.event === 'wrapper-start').length, 1, 'startup timeout cannot silently retry the same attempt');
  assert.deepEqual(captured.filter(event => event.event === 'outbound').map(event => event.message.method), ['initialize'],
    'an uninitialized attempt must not send shutdown, exit, initialized or ordinary work');
}

async function retirePendingStartup(extension, replacement) {
  const activation = observed(extension.activate());
  const held = await waitFor(items => items.find(event => event.event === 'initialize-response-withheld'), 'withheld initialize before retirement');
  if (!replacement) await deactivate(extension);
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_ALTERNATE_PATH, vscode.ConfigurationTarget.Global,
  );
  if (replacement) {
    await waitFor(items => items.find(event => event.session !== held.session && event.event === 'outbound'
      && event.message.method === 'initialized'), 'replacement while old initialization is missing');
  }
  // Activation may finish on the reconciled current configuration; unlike a
  // business operation it is not itself a language request bound to the old peer.
  await bounded(activation, 'activation interrupted by retirement', 10_000);
  await deactivate(extension);
  await waitForOwnedProcessCleanup();
  const captured = await events();
  const old = captured.filter(event => event.session === held.session && event.event === 'outbound');
  assert.deepEqual(old.map(event => event.message.method), ['initialize'],
    'retiring Starting cannot submit shutdown, exit or late bootstrap work');
  assert.equal(captured.filter(event => event.event === 'wrapper-start').length, replacement ? 2 : 1,
    'only the current configuration may start; deactivation is terminal');
}

async function openLifecycleProbe(filename) {
  const workspace = vscode.workspace.workspaceFolders[0];
  const uri = vscode.Uri.joinPath(workspace.uri, filename);
  await fs.writeFile(uri.fsPath, '{}');
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);
  return document;
}

async function startupCoalescing(extension) {
  const activation = observed(extension.activate());
  const a = await waitFor(items => items.find(event => event.event === 'initialize-response-withheld'), 'obsolete A initialize');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_ALTERNATE_PATH, vscode.ConfigurationTarget.Global,
  );
  const b = await waitFor(items => items.find(event => event.session !== a.session
    && event.event === 'initialize-response-withheld'), 'obsolete B initialize while A retires');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_FINAL_PATH, vscode.ConfigurationTarget.Global,
  );
  const c = await waitFor(items => {
    const current = items.find(event => event.event === 'wrapper-start'
      && event.executable === process.env.DAML_LSP_FINAL_PATH);
    return current && items.find(event => event.session === current.session
      && event.event === 'outbound' && event.message.method === 'initialized');
  }, 'current C initialize after A and B are obsolete');
  await bounded(activation, 'activation across A to B to C', 10_000);
  const document = await openLifecycleProbe('current-c.daml');
  const context = await bounded(vscode.commands.executeCommand(
    'definitivelyAssertedDaml.useSyntax', { uri: document.uri.toString() },
  ), 'language work on current C', 10_000);
  assert.ok(Number.isInteger(context.generation) && context.generation > 0);
  await deactivate(extension);
  await waitForOwnedProcessCleanup();
  const captured = await events();
  assert.equal(captured.filter(event => event.event === 'wrapper-start').length, 3,
    'three configuration intentions must not produce duplicate or obsolete restarts');
  for (const obsolete of [a, b]) {
    assert.deepEqual(captured.filter(event => event.session === obsolete.session && event.event === 'outbound')
      .map(event => event.message.method), ['initialize'],
    'obsolete A and B cannot finish bootstrap, admit application work or send pre-initialization shutdown');
  }
  assert.ok(captured.some(event => event.session === c.session && event.event === 'outbound'
    && event.message.method === 'workspace/executeCommand'
    && event.message.params?.arguments?.[0]?.uri === document.uri.toString()),
  'the acknowledged language operation must belong to current C');
}

async function retirementFailure(extension) {
  await bounded(extension.activate(), 'activation before deliberate retirement failure', 10_000);
  const document = await openLifecycleProbe('failed-retirement.daml');
  const select = () => vscode.commands.executeCommand(
    'definitivelyAssertedDaml.useSyntax', { uri: document.uri.toString() },
  );
  const initial = await bounded(select(), 'real language work before retirement failure', 10_000);
  assert.ok(Number.isInteger(initial.generation) && initial.generation > 0);
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_ALTERNATE_PATH, vscode.ConfigurationTarget.Global,
  );
  const fault = await waitFor(items => items.find(event => event.event === 'shutdown-fault-response'),
    'genuine native error for faulted shutdown');
  assert.equal(fault.message.error?.code, -32601, 'the native peer must reject the unsupported method');
  const afterFailure = await bounded(observed(select()), 'context after failed retirement', 10_000);
  assert.equal(afterFailure.status, 'rejected', 'a context command cannot recover from unknown retirement or report language success');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_FINAL_PATH, vscode.ConfigurationTarget.Global,
  );
  const afterConfiguration = await bounded(observed(select()), 'context after another configuration intention', 10_000);
  assert.equal(afterConfiguration.status, 'rejected', 'another configuration cannot erase the failed retirement outcome');
  const deactivation = await observed(deactivate(extension));
  assert.equal(deactivation.status, 'rejected', 'deactivation must retain the explicit failed-retirement outcome');
  assert.equal(deactivation.reason.code, fault.message.error.code);
  await waitForOwnedProcessCleanup();
  const captured = await events();
  assert.equal(captured.filter(event => event.event === 'wrapper-start').length, 1,
    'neither configuration nor context commands may start replacement after failed retirement');
  const outbound = captured.filter(event => event.session === fault.session && event.event === 'outbound')
    .map(event => event.message);
  const shutdown = outbound.findIndex(message => message.method === 'shutdown');
  assert.ok(shutdown >= 0);
  assert.equal(outbound[shutdown].id, fault.message.id, 'native shutdown failure must be correlated to the retiring client');
  assert.deepEqual(outbound.slice(shutdown + 1), [], 'failed shutdown cannot authorize exit or late ordinary messages');
}

async function crashLineage(extension) {
  await bounded(extension.activate(), 'initial activation', 10_000);
  const workspace = vscode.workspace.workspaceFolders[0];
  const uri = vscode.Uri.joinPath(workspace.uri, 'crash-lineage.daml');
  await fs.writeFile(uri.fsPath, '{}');
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);
  const ready = () => vscode.commands.executeCommand('definitivelyAssertedDaml.useSyntax', { uri: uri.toString() });
  // The stock default permits four replacements in one three-minute lineage.
  // Crash only after successful language work, never by racing initialization.
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const initialized = await waitFor(items => {
      const sessions = items.filter(event => event.event === 'wrapper-start');
      const session = sessions[attempt]?.session;
      return session && items.find(event => event.session === session && event.event === 'outbound' && event.message.method === 'initialized');
    }, `initialized attempt ${attempt + 1}`);
    const context = await bounded(ready(), `language work before crash ${attempt + 1}`, 10_000);
    assert.ok(Number.isInteger(context.generation));
    await fs.writeFile(process.env.DAML_LSP_CRASH_CONTROL, String(initialized.session));
    await waitFor(items => items.find(event => event.session === initialized.session && event.event === 'child-exit'), `crash ${attempt + 1}`);
  }
  // This is a bounded negative observation, not a scheduling delay that creates
  // the interleaving: each of the five closes above has its own real-peer barrier.
  const until = Date.now() + 5000;
  do {
    assert.equal((await events()).filter(event => event.event === 'wrapper-start').length, 5,
      'constructing replacement clients must not reset the stock crash-restart history');
    await new Promise(resolve => setTimeout(resolve, 25));
  } while (Date.now() < until);

  // A deliberate new configuration does establish a new lineage, as before.
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_ALTERNATE_PATH, vscode.ConfigurationTarget.Global,
  );
  await waitFor(items => {
    const session = items.filter(event => event.event === 'wrapper-start')[5]?.session;
    return session && items.find(event => event.session === session && event.event === 'outbound' && event.message.method === 'initialized');
  }, 'explicit new configuration after exhausted crash lineage');
  await bounded(ready(), 'new lineage context command', 10_000);
  await deactivate(extension);
  assert.equal((await events()).filter(event => event.event === 'wrapper-start').length, 6);
}

async function run() {
  assert.equal(vscode.workspace.isTrusted, true);
  const extension = vscode.extensions.getExtension(extensionId);
  assert.ok(extension, 'installed VSIX must be visible');
  if (process.env.DAML_VSCODE_SCENARIO === 'startup-timeout') await startupTimeout(extension);
  else if (process.env.DAML_VSCODE_SCENARIO === 'startup-replacement') await retirePendingStartup(extension, true);
  else if (process.env.DAML_VSCODE_SCENARIO === 'startup-deactivation') await retirePendingStartup(extension, false);
  else if (process.env.DAML_VSCODE_SCENARIO === 'crash-lineage') await crashLineage(extension);
  else if (process.env.DAML_VSCODE_SCENARIO === 'startup-coalescing') await startupCoalescing(extension);
  else if (process.env.DAML_VSCODE_SCENARIO === 'retirement-failure') await retirementFailure(extension);
  else throw new Error('Unknown adversarial lifecycle scenario');
}
module.exports = { run };
