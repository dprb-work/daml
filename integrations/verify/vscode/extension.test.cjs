const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const vscode = require('vscode');

const extensionId = 'dprb-work.definitively-asserted-daml';
const invalidSource = '"😀é" ?';
const validSource = '{message:"😀é"}';
const formattedSource = '{ message: "😀é" }\n';

function waitForDiagnostics(uri, predicate, timeoutMs = 10_000) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      subscription.dispose();
      reject(new Error(`Timed out waiting for diagnostics for ${uri.toString()}`));
    }, timeoutMs);
    const subscription = vscode.languages.onDidChangeDiagnostics(event => {
      if (!event.uris.some(candidate => candidate.toString() === uri.toString())) return;
      const diagnostics = vscode.languages.getDiagnostics(uri);
      if (!predicate(diagnostics)) return;
      clearTimeout(timeout);
      subscription.dispose();
      resolve(diagnostics);
    });
    const initial = vscode.languages.getDiagnostics(uri);
    if (predicate(initial)) {
      clearTimeout(timeout);
      subscription.dispose();
      resolve(initial);
    }
  });
}
function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error.code === 'ESRCH') return false;
    throw error;
  }
}

function waitForProcessExit(pid, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + timeoutMs;
    const check = () => {
      if (!processIsAlive(pid)) {
        resolve();
      } else if (Date.now() >= deadline) {
        reject(new Error(`Process ${pid} remained alive after client restart`));
      } else {
        setTimeout(check, 100);
      }
    };
    check();
  });
}

async function processId(path) {
  const pid = Number(await fs.readFile(path, 'utf8'));
  assert.ok(Number.isSafeInteger(pid) && pid > 0, `Invalid process ID in ${path}`);
  return pid;
}
async function recordLifecycleStage(stage) {
  await fs.appendFile(process.env.DAML_LSP_STAGE_FILE, `${JSON.stringify({ event: 'test-stage', stage })}\n`);
}

async function waitForWireEvent(eventName, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const lines = (await fs.readFile(process.env.DAML_LSP_STATUS_FILE, 'utf8')).split('\n');
    // A concurrent append can expose an incomplete final line; complete records
    // alone are observation barriers, never a sleep chosen to create the race.
    const events = lines.slice(0, -1).filter(Boolean).map(line => JSON.parse(line));
    const event = events.find(item => item.event === eventName);
    if (event) return event;
    if (Date.now() >= deadline) throw new Error(`No wire event: ${eventName}`);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}

async function bounded(promise, label, timeoutMs = 10_000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), timeoutMs);
    })]);
  } finally { clearTimeout(timer); }
}


async function run() {
  const scenario = process.env.DAML_VSCODE_SCENARIO;
  assert.ok(scenario === 'trusted' || scenario === 'untrusted', `Unknown VS Code host scenario: ${scenario}`);
  const extension = vscode.extensions.getExtension(extensionId);
  assert.ok(extension, `Installed VSIX ${extensionId} is not visible in the extension host`);
  const workspace = vscode.workspace.workspaceFolders?.[0];
  assert.ok(workspace, 'Test workspace was not opened');

  if (scenario === 'untrusted') {
    assert.equal(vscode.workspace.isTrusted, false, 'The untrusted Electron scenario must remain untrusted');
    await extension.activate();
    const file = vscode.Uri.joinPath(workspace.uri, 'probe.daml');
    await fs.writeFile(file.fsPath, invalidSource, 'utf8');
    const document = await vscode.workspace.openTextDocument(file);
    assert.equal(document.languageId, 'definitively-asserted-daml');
    await vscode.window.showTextDocument(document);
    assert.equal(vscode.workspace.isTrusted, false, 'Opening a DAML document must not grant trust');
    await vscode.commands.executeCommand('workbench.action.closeAllEditors');
    return;
  }

  assert.equal(vscode.workspace.isTrusted, true, 'The trusted Electron scenario must be trusted');
  await recordLifecycleStage('initial-activation');
  await extension.activate();
  const file = vscode.Uri.joinPath(workspace.uri, 'probe.daml');
  await fs.writeFile(file.fsPath, invalidSource, 'utf8');
  const invalid = await vscode.workspace.openTextDocument(file);
  assert.equal(invalid.languageId, 'definitively-asserted-daml');
  const editor = await vscode.window.showTextDocument(invalid);

  await recordLifecycleStage('initial-diagnostics');
  const diagnostics = await waitForDiagnostics(file, items => items.length > 0);
  assert.equal(diagnostics.length, 1, 'expected one syntax diagnostic');
  assert.deepEqual(diagnostics[0].range, new vscode.Range(0, 6, 0, 7), 'diagnostic must use the LSP UTF-16 range after supplementary Unicode');

  assert.equal(await editor.edit(builder => builder.replace(new vscode.Range(invalid.positionAt(0), invalid.positionAt(invalid.getText().length)), validSource)), true);
  const cleared = await waitForDiagnostics(file, items => items.length === 0);
  assert.deepEqual(cleared, [], 'replacing invalid text must clear diagnostics');

  const format = () => vscode.commands.executeCommand('vscode.executeFormatDocumentProvider', invalid.uri, { tabSize: 2, insertSpaces: true });
  const edits = await format();
  assert.ok(Array.isArray(edits), 'format provider did not return edits');
  const edit = new vscode.WorkspaceEdit();
  for (const item of edits) edit.replace(invalid.uri, item.range, item.newText);
  assert.equal(await vscode.workspace.applyEdit(edit), true, 'editor rejected formatting edit');
  assert.equal(invalid.getText(), formattedSource);
  const noOpEdits = await format();
  assert.ok(noOpEdits === undefined || (Array.isArray(noOpEdits) && noOpEdits.length === 0), 'canonical formatting must return no edits');
  assert.equal(invalid.getText(), formattedSource, 'canonical formatting must leave the document unchanged');

  assert.equal(await editor.edit(builder => builder.replace(new vscode.Range(invalid.positionAt(0), invalid.positionAt(invalid.getText().length)), invalidSource)), true);
  const before = invalid.getText();
  let reported = false;
  try {
    const badEdits = await vscode.commands.executeCommand('vscode.executeFormatDocumentProvider', invalid.uri, { tabSize: 2, insertSpaces: true });
    reported = badEdits === undefined || (Array.isArray(badEdits) && badEdits.length === 0);
  } catch {
    reported = true;
  }
  assert.ok(reported, 'invalid source formatting must report failure or return no edits');
  assert.equal(invalid.getText(), before, 'invalid formatting must not destructively change editor text');
  const wrapperPid = await processId(process.env.DAML_LSP_WRAPPER_PID_FILE);
  const serverPid = await processId(process.env.DAML_LSP_CHILD_PID_FILE);
  assert.ok(processIsAlive(wrapperPid), 'the user-configured wrapper must remain alive after the real language server starts');
  const oldServerExit = Promise.all([waitForProcessExit(wrapperPid), waitForProcessExit(serverPid)]);
  // The real server handles this request, but the observer withholds its result.
  // Replacement must not hold the lifecycle queue awaiting this business reply.
  const context = vscode.commands.executeCommand('definitivelyAssertedDaml.useSyntax', { uri: invalid.uri.toString() })
    .then(value => ({ status: 'fulfilled', value }), reason => ({ status: 'rejected', reason }));
  await waitForWireEvent('context-response-withheld');
  await recordLifecycleStage('restart');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath',
    process.env.DAML_LSP_REPLACEMENT_PATH,
    vscode.ConfigurationTarget.Global,
  );
  await oldServerExit;
  const retiredContext = await bounded(context, 'context operation from retired session');
  assert.equal(retiredContext.status, 'rejected', 'a missing old-session response must settle locally, not become success');

  const restartedFile = vscode.Uri.joinPath(workspace.uri, 'restarted.daml');
  await fs.writeFile(restartedFile.fsPath, invalidSource, 'utf8');
  const restartedDocument = await vscode.workspace.openTextDocument(restartedFile);
  await vscode.window.showTextDocument(restartedDocument);
  const restartedDiagnostics = await waitForDiagnostics(restartedFile, items => items.length > 0);
  assert.equal(restartedDiagnostics.length, 1, 'the configured released server must start after the wrapper is terminated');
  const currentContext = await bounded(vscode.commands.executeCommand(
    'definitivelyAssertedDaml.useSyntax', { uri: restartedDocument.uri.toString() },
  ), 'replacement context command');
  assert.ok(Number.isInteger(currentContext.generation) && currentContext.generation > 0,
    'old context retirement must not invalidate the replacement session');

  await vscode.commands.executeCommand('workbench.action.closeAllEditors');
  await recordLifecycleStage('deactivation');
  // Invoke the extension's public deactivation export, not private session state.
  const installed = require(path.join(extension.extensionPath, extension.packageJSON.main));
  await bounded(installed.deactivate(), 'installed extension deactivation');
  await vscode.workspace.getConfiguration('definitivelyAssertedDaml').update(
    'serverPath', process.env.DAML_LSP_WRAPPER_PATH, vscode.ConfigurationTarget.Global,
  );
  await assert.rejects(vscode.commands.executeCommand(
    'definitivelyAssertedDaml.useSyntax', { uri: restartedDocument.uri.toString() },
  ), 'a command after deactivation cannot activate a new session');
  // Drain an already-delivered configuration callback through the same public
  // lifecycle boundary; the outer runner checks that it spawned no third peer.
  await bounded(installed.deactivate(), 'deactivation after queued configuration');
}

async function runWithLaunchDiagnostics() {
  try {
    await run();
  } catch (error) {
    try {
      const configuration = vscode.workspace.getConfiguration('definitivelyAssertedDaml');
      console.error(JSON.stringify({
        event: 'lifecycle-extension-host-environment',
        nodeVersion: process.version,
        execPath: process.execPath,
        scenario: process.env.DAML_VSCODE_SCENARIO,
        trusted: vscode.workspace.isTrusted,
        selectedServerPath: configuration.get('serverPath'),
        serverPathConfiguration: configuration.inspect('serverPath'),
        environment: Object.fromEntries(['PATH', 'NODE_OPTIONS', 'NODE_PATH', 'ELECTRON_RUN_AS_NODE', 'ELECTRON_NO_ASAR']
          .map(name => [name, process.env[name] ?? null])),
      }, null, 2));
    } catch (diagnosticError) {
      console.error('Could not collect launch diagnostics:', diagnosticError);
    }
    throw error;
  }
}

module.exports = { run: runWithLaunchDiagnostics };
