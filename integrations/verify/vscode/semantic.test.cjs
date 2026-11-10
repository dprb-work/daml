const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const vscode = require('vscode');

const extensionId = 'dprb-work.definitively-asserted-daml';
const command = kind => `definitivelyAssertedDaml.use${kind}`;
const completion = (uri, line, character) => vscode.commands.executeCommand(
  'vscode.executeCompletionItemProvider', uri, new vscode.Position(line, character), undefined, 100,
);
const hover = (uri, line, character) => vscode.commands.executeCommand(
  'vscode.executeHoverProvider', uri, new vscode.Position(line, character),
);
const definition = (uri, line, character) => vscode.commands.executeCommand(
  'vscode.executeDefinitionProvider', uri, new vscode.Position(line, character),
);
const verifyTrace = process.env.DAML_VERIFY_TRACE === '1';
let retargetTraceEvents = 0;
function traceRetarget(stage, document, alias, middle, error) {
  if (!verifyTrace || retargetTraceEvents >= 32) return;
  retargetTraceEvents += 1;
  const time = Date.now();
  const links = {};
  for (const [name, file] of [['Alias', alias], ['Middle', middle]]) {
    try {
      links[name] = { path: file, target: require('node:fs').readlinkSync(file) };
    } catch (failure) {
      links[name] = { path: file, error: { code: failure.code ?? null, message: failure.message } };
    }
  }
  console.error(JSON.stringify({
    event: 'semantic-retarget-trace', stage, time, isoTime: new Date(time).toISOString(),
    uri: document.uri.toString(), version: document.version, closed: document.isClosed,
    links, error: error?.stack ?? (error === undefined ? undefined : String(error)),
  }));
}

// Subscribe before the mutation. An already empty diagnostic collection is not proof
// that the accepted edit/context/watch notification has been processed.
async function diagnosticsAfter(uri, predicate, action) {
  const callsite = new Error('Diagnostic wait registered here').stack;
  let subscription;
  let timer;
  const changed = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`No matching diagnostic event for ${uri}: ${JSON.stringify(vscode.languages.getDiagnostics(uri))}\n${callsite}`)), 15_000);
    subscription = vscode.languages.onDidChangeDiagnostics(event => {
      if (!event.uris.some(candidate => candidate.toString() === uri.toString())) return;
      const items = vscode.languages.getDiagnostics(uri);
      if (predicate(items)) resolve(items);
    });
  });
  // Install a rejection handler immediately if action itself is asynchronous.
  changed.catch(() => {});
  try {
    await action();
    return await changed;
  } finally {
    clearTimeout(timer);
    subscription.dispose();
  }
}
async function replace(document, text) {
  const edit = new vscode.WorkspaceEdit();
  edit.replace(document.uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), text);
  assert.equal(await vscode.workspace.applyEdit(edit), true);
  assert.equal(document.getText(), text);
}
async function openEditorDocument(uri) {
  // workspace.openTextDocument pins the model in VS Code's extension-open
  // reference cache for up to three minutes. Open a real tab instead, so closing
  // its last editor can actually dispose the document and send didClose.
  let subscription;
  let timer;
  let acceptEditor;
  const opened = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`Editor did not open: ${uri}`)), 15_000);
    acceptEditor = editor => {
      if (editor?.document.uri.toString() === uri.toString()) resolve(editor.document);
    };
    subscription = vscode.window.onDidChangeActiveTextEditor(acceptEditor);
  });
  opened.catch(() => {});
  try {
    await vscode.commands.executeCommand('vscode.open', uri, { preview: false });
    acceptEditor(vscode.window.activeTextEditor);
    return await opened;
  } finally {
    clearTimeout(timer);
    subscription.dispose();
  }
}
async function closeReverting(document) {
  await vscode.window.showTextDocument(document, { preview: false });
  await vscode.commands.executeCommand('workbench.action.files.revert');
  let subscription;
  let timer;
  const closed = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`Document did not close: ${document.uri}`)), 15_000);
    subscription = vscode.workspace.onDidCloseTextDocument(item => {
      if (item.uri.toString() === document.uri.toString()) resolve();
    });
  });
  closed.catch(() => {});
  try {
    await vscode.commands.executeCommand('workbench.action.closeActiveEditor');
    await closed;
  } finally {
    clearTimeout(timer);
    subscription.dispose();
  }
  assert.equal(document.isClosed, true);
}
function findItem(result, name) {
  assert.ok(result && Array.isArray(result.items), `No completion list for ${name}`);
  const item = result.items.find(item => (typeof item.label === 'string' ? item.label : item.label.label) === name);
  assert.ok(item, `Missing completion ${name}: ${JSON.stringify(result.items)}`);
  return item;
}
function rangeOf(item) { return item.range?.replacing ?? item.range; }
function hoverText(items) {
  return (items ?? []).flatMap(item => item.contents.map(content => typeof content === 'string' ? content : content.value)).join('\n');
}
function hasLocation(items, uri, range) {
  return (items ?? []).some(item => {
    const target = item.targetUri ?? item.uri;
    const selection = item.targetSelectionRange ?? item.range;
    return target.toString() === uri.toString() && selection.isEqual(range);
  });
}
async function checkSymlinkDotContext(workspace, root) {
  const base = path.join(root, 'uri-context');
  const a = path.join(base, 'a');
  const b = path.join(base, 'b');
  const wrongBundle = path.join(a, 'bundle');
  const rightBundle = path.join(b, 'bundle');
  const child = path.join(b, 'child');
  await Promise.all([
    fs.mkdir(wrongBundle, { recursive: true }),
    fs.mkdir(rightBundle, { recursive: true }),
    fs.mkdir(child, { recursive: true }),
  ]);
  await fs.symlink(child, path.join(a, 'link'), 'dir');
  await Promise.all([
    fs.writeFile(path.join(wrongBundle, 'daml.bundle'), '{}\n', 'utf8'),
    fs.writeFile(path.join(rightBundle, 'daml.bundle'), '{}\n', 'utf8'),
    fs.writeFile(path.join(wrongBundle, 'Config.daml'), 'closed !Object {\n endpoint: !Int\n}\n', 'utf8'),
    fs.writeFile(path.join(rightBundle, 'Config.daml'), 'closed !Object {\n endpoint: !String\n}\n', 'utf8'),
  ]);
  // Preserve symlink/.. ordering in both paths. Lexical path normalization would
  // select a/bundle, whereas the real filesystem walk selects b/bundle.
  assert.equal(await fs.realpath(`${a}/link/../bundle`), await fs.realpath(rightBundle));
  const rawBundleRoot = `${vscode.Uri.file(a).toString()}/link/../bundle`;
  const uri = vscode.Uri.joinPath(workspace, 'symlink-context.daml');
  await fs.writeFile(uri.fsPath, '{\n endpoint: 42\n}', 'utf8');
  const document = await openEditorDocument(uri);
  const errors = await diagnosticsAfter(uri, items => items.length > 0, () => vscode.commands.executeCommand(command('Configuration'), {
    uri: uri.toString(), bundleRoot: rawBundleRoot, expected: 'self.Config',
  }));
  assert.ok(errors.some(item => item.range.isEqual(new vscode.Range(1, 11, 1, 13))), 'Filesystem-resolved String bundle must reject the supplied number');
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(document, '{\n endpoint: "ok"\n}'));
  assert.match(hoverText(await hover(uri, 1, 4)), /string/i);
  const targets = await definition(uri, 1, 4);
  assert.ok(hasLocation(targets, vscode.Uri.file(path.join(rightBundle, 'Config.daml')), new vscode.Range(1, 1, 1, 9)), 'The raw symlink/.. context must navigate into b/bundle, never lexically normalized a/bundle');
  await closeReverting(document);
}
function definitionSymlinkRetargetCases(root) {
  // The runner seeded every probe file before Code launched. This function only
  // describes those fixtures; no setup events can leak into measured retargets.
  const bundle = path.join(root, 'providers');
  const cases = [];
  for (const [name, prefix, chained] of [
    ['file-retarget', 'DirectRetarget', false],
    ['file-retarget-chain', 'ChainedRetarget', true],
  ]) {
    const stringName = `${prefix}StringDef.daml`;
    const intName = `${prefix}IntDef.daml`;
    const middleName = `${prefix}Middle.daml`;
    const aliasStem = `${prefix}Alias`;
    const stringDefinition = path.join(bundle, stringName);
    const intDefinition = path.join(bundle, intName);
    const alias = path.join(bundle, `${aliasStem}.daml`);
    const middle = path.join(bundle, middleName);
    const pending = path.join(bundle, `${prefix}.pending`);
    const stringSource = '!String\n';
    const intSource = '!Int\n';
    const aliasTarget = chained ? middleName : stringName;
    cases.push({
      name, chained, bundle, stringName, intName, aliasStem, stringDefinition,
      intDefinition, alias, middle, pending, stringSource, intSource, aliasTarget,
    });
  }
  return cases;
}
async function checkAtomicDefinitionSymlinkRetarget(workspace, cases) {
  for (const {
    name, chained, bundle, stringName, intName, aliasStem, stringDefinition,
    intDefinition, alias, middle, pending, stringSource, intSource, aliasTarget,
  } of cases) {
    const entry = chained ? middle : alias;
    const uri = vscode.Uri.joinPath(workspace, `${name}.daml`);
    const source = `!self.${aliasStem} "kept"\n`;
    const document = await openEditorDocument(uri);
    assert.equal(document.getText(), source, 'Probe configuration must be preseeded before Code launches');
    await vscode.commands.executeCommand(command('Configuration'), {
      uri: uri.toString(), bundleRoot: vscode.Uri.file(bundle).toString(), expected: `self.${aliasStem}`,
    });
    traceRetarget('context-complete', document, alias, middle);
    const valuePosition = document.positionAt(source.indexOf('"') + 1);
    assert.match(hoverText(await hover(uri, valuePosition.line, valuePosition.character)), /string/i);
    traceRetarget('hover-complete', document, alias, middle);
    const version = document.version;
    const definitions = [alias, stringDefinition, intDefinition];
    if (chained) definitions.push(middle);
    for (const file of definitions) {
      assert.equal(vscode.workspace.textDocuments.some(item => item.uri.toString() === vscode.Uri.file(file).toString()), false, 'Retarget proof requires all definition buffers to remain unopened');
    }
    // Direct and chained cases change only one file-leaf link entry. In the
    // chained case Alias itself remains fixed, so Alias+old-target watches fail.
    const errors = await diagnosticsAfter(uri, items => items.length > 0, async () => {
      traceRetarget('retarget-int-start', document, alias, middle);
      await fs.symlink(intName, pending, 'file');
      await fs.rename(pending, entry);
      traceRetarget('retarget-int-rename-complete', document, alias, middle);
    }).catch(error => {
      traceRetarget('retarget-int-wait-failed', document, alias, middle, error);
      throw error;
    });
    assert.ok(errors.some(item => /integer/i.test(item.message)), `${name} must apply the Int contract without editing or requesting the configuration`);
    traceRetarget('retarget-int-error-observed', document, alias, middle);
    if (chained) assert.equal(await fs.readlink(alias), aliasTarget);
    await diagnosticsAfter(uri, items => items.length === 0, async () => {
      traceRetarget('retarget-string-start', document, alias, middle);
      await fs.symlink(stringName, pending, 'file');
      await fs.rename(pending, entry);
      traceRetarget('retarget-string-rename-complete', document, alias, middle);
    }).catch(error => {
      traceRetarget('retarget-string-wait-failed', document, alias, middle, error);
      throw error;
    });
    traceRetarget('retarget-string-clear-observed', document, alias, middle);
    assert.equal(await fs.readlink(alias), aliasTarget);
    assert.equal(document.version, version);
    assert.equal(document.getText(), source);
    assert.equal(await fs.readFile(stringDefinition, 'utf8'), stringSource);
    assert.equal(await fs.readFile(intDefinition, 'utf8'), intSource);
    await closeReverting(document);
    traceRetarget('case-complete', document, alias, middle);
  }
}

async function run() {
  const extension = vscode.extensions.getExtension(extensionId);
  assert.ok(extension, `Built source VSIX ${extensionId} was not installed`);
  assert.equal(vscode.workspace.isTrusted, true, 'Semantic filesystem tests require a trusted host');
  const root = process.env.DAML_SEMANTIC_FIXTURE;
  assert.ok(root);
  const workspace = vscode.Uri.file(path.join(root, 'workspace'));
  assert.equal(vscode.workspace.workspaceFolders[0].uri.toString(), workspace.toString());
  const uri = vscode.Uri.joinPath(workspace, 'config.daml');
  const provider = vscode.Uri.file(path.join(root, 'providers', 'Http.daml'));
  const service = vscode.Uri.joinPath(workspace, 'app', 'Service.daml');
  const manifest = vscode.Uri.joinPath(workspace, 'daml.bundle');
  const source = await fs.readFile(uri.fsPath, 'utf8');
  const http = await fs.readFile(provider.fsPath, 'utf8');
  const bundle = await fs.readFile(manifest.fsPath, 'utf8');
  assert.equal(vscode.workspace.getWorkspaceFolder(provider), undefined, 'Watch target must really be outside workspace');
  const retargetCases = definitionSymlinkRetargetCases(root);
  await extension.activate();
  let document;
  await diagnosticsAfter(uri, items => items.length > 0, async () => {
    document = await openEditorDocument(uri);
    await replace(document, '{ broken:');
  });
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(document, source));
  assert.equal(document.languageId, 'definitively-asserted-daml');
  assert.deepEqual(await hover(uri, 3, 6), [], 'Unselected documents must remain syntax-only');
  await vscode.commands.executeCommand(command('Configuration'), {
    uri: uri.toString(), bundleRoot: workspace.toString(), expected: 'app.Service',
  });
  const endpointHover = hoverText(await hover(uri, 3, 6));
  assert.match(endpointHover, /string/i, 'Hover must expose the endpoint contract');
  const adapterHover = hoverText(await hover(uri, 2, 4));
  assert.match(adapterHover, /30/);
  assert.match(adapterHover, /default/i, 'Hover must distinguish a completed default');
  const endpointTargets = await definition(uri, 3, 6);
  assert.ok(hasLocation(endpointTargets, provider, new vscode.Range(1, 2, 1, 10)), `Field must navigate to its declaration: ${JSON.stringify(endpointTargets)}`);
  const serviceTargets = await definition(uri, 0, 5);
  assert.ok(serviceTargets.some(target => (target.targetUri ?? target.uri).toString() === service.toString()
    && (target.targetSelectionRange ?? target.range).start.isEqual(new vscode.Position(0, 0))), 'Tag must navigate to the actual native source');

  const prefix = '!app.Service {\n  name: "demo"\n  adapter: !providers.Http {\n    endpoint: "https://example.com"\n    time';
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, prefix));
  const fields = await completion(uri, 4, 8);
  const timeout = findItem(fields, 'timeout');
  assert.ok(rangeOf(timeout).isEqual(new vscode.Range(4, 4, 4, 8)), 'Completion must replace precisely the typed field prefix');
  assert.match(JSON.stringify(timeout), /30/, 'Completion must carry the timeout default');
  assert.ok(!fields.items.some(item => item.label === 'endpoint'), 'Do not suggest already supplied fields');
  assert.ok(vscode.languages.getDiagnostics(uri).length > 0, 'Completion must not erase incomplete syntax diagnostics');
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, '!providers.'));
  const tags = await completion(uri, 0, 11);
  assert.ok(rangeOf(findItem(tags, 'providers.Http')).isEqual(new vscode.Range(0, 1, 0, 11)));
  findItem(tags, 'providers.Unloaded');
  assert.ok(!tags.items.some(item => String(item.label).startsWith('contracts.')), 'Private dependency imports must not leak');
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, '!'));
  const visible = await completion(uri, 0, 1);
  findItem(visible, 'app.Service');
  assert.ok(!visible.items.some(item => {
    const label = typeof item.label === 'string' ? item.label : item.label.label;
    return label.startsWith('contracts.');
  }), 'Entry scope must exclude private aliases even without a prefix filter');
  await vscode.commands.executeCommand(command('Configuration'), {
    uri: uri.toString(), bundleRoot: workspace.toString(),
  });
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, '!app.Worker {\n'));
  const worker = await completion(uri, 1, 0);
  assert.match(JSON.stringify(findItem(worker, 'name')), /required/i);
  assert.match(JSON.stringify(findItem(worker, 'note')), /optional/i);
  assert.match(JSON.stringify(findItem(worker, 'retries')), /3/);
  await vscode.commands.executeCommand(command('Configuration'), {
    uri: uri.toString(), bundleRoot: workspace.toString(), expected: 'app.Service',
  });
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(document, source));

  const bad = source.replace('"https://example.com"', '42');
  const errors = await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, bad));
  assert.ok(errors.some(item => item.range.isEqual(new vscode.Range(3, 14, 3, 16))), 'Semantic failure must point to the supplied number');
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(document, source));
  const unicodeBad = source.replace('endpoint: "https://example.com"', 'endpoint: "😀é"\n    timeout: "😀é"');
  const unicodeErrors = await diagnosticsAfter(uri, items => items.length > 0, () => replace(document, unicodeBad));
  assert.ok(unicodeErrors.some(item => item.range.isEqual(new vscode.Range(4, 13, 4, 18))), 'Semantic Unicode value spans must use VS Code UTF-16 units');
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(document, unicodeBad.replace('timeout: "😀é"', 'timeout: 30')));
  assert.match(hoverText(await hover(uri, 3, 17)), /😀é/, 'Inbound hover position after a supplementary character must reach the supplied value');
  await replace(document, source);
  assert.match(hoverText(await hover(uri, 3, 6)), /string/i);

  // Open buffer, not saved bytes, must drive dependent analysis.
  const providerDocument = await openEditorDocument(provider);
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(providerDocument, http.replace('endpoint: !String', 'endpoint: !Int')));
  assert.equal(await fs.readFile(provider.fsPath, 'utf8'), http, 'Definition test must remain unsaved');
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(providerDocument, http));
  await vscode.commands.executeCommand(command('Definition'), { uri: provider.toString() });
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(providerDocument, http.replace('!String', '!NoSuchNativeType')));
  const referencedErrors = vscode.languages.getDiagnostics(uri);
  assert.ok(referencedErrors.some(item => item.relatedInformation?.some(info => info.location.uri.toString() === provider.toString())), 'Referenced failure must identify its physical source');
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(providerDocument, http));

  const manifestDocument = await openEditorDocument(manifest);
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(manifestDocument, bundle.replace('../providers', '../missing')));
  assert.equal(await fs.readFile(manifest.fsPath, 'utf8'), bundle);
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(manifestDocument, bundle));
  await diagnosticsAfter(uri, items => items.length > 0, () => replace(manifestDocument, '{ broken:'));
  await diagnosticsAfter(uri, items => items.length === 0, () => replace(manifestDocument, bundle));

  // Close all dependency buffers before editing disk: otherwise their current open
  // snapshots correctly mask disk edits. Retain only the configuration editor.
  await closeReverting(providerDocument);
  await closeReverting(manifestDocument);
  await vscode.window.showTextDocument(document, { preview: false });
  assert.equal(vscode.workspace.textDocuments.some(item => item.uri.toString() === provider.toString() && !item.isClosed), false, 'External watch proof requires no open definition overlay');
  // The real provider round trip orders backend didClose processing before the
  // disk edit. A queued close must not masquerade as dependency watcher proof.
  assert.match(hoverText(await hover(uri, 3, 6)), /string/i);
  // Rejected replacement must not drop the established semantic selection or
  // its external dependency watcher. Do not install a valid context afterward:
  // the next disk edits must still be observed through the existing membership.
  await assert.rejects(() => vscode.commands.executeCommand(command('Configuration'), {
    uri: uri.toString(), bundleRoot: 'https://example.invalid/bundle',
  }));
  assert.match(hoverText(await hover(uri, 3, 6)), /string/i);
  await diagnosticsAfter(uri, items => items.length > 0, () => fs.writeFile(provider.fsPath, http.replace('endpoint: !String', 'endpoint: !Int'), 'utf8'));
  await diagnosticsAfter(uri, items => items.length === 0, () => fs.writeFile(provider.fsPath, http, 'utf8'));
  // No completion/hover request is issued between the disk edit and diagnostic event:
  // the extension's real external dependency watcher is the only refresh trigger.
  // Keep Service selected: its proven Http/manifest memberships pin this same
  // canonical outside-workspace directory backend throughout both leaf probes.
  await checkAtomicDefinitionSymlinkRetarget(workspace, retargetCases);

  await vscode.commands.executeCommand(command('Syntax'), { uri: uri.toString() });
  assert.deepEqual(await hover(uri, 3, 6), []);
  await replace(document, bad);
  assert.deepEqual(await hover(uri, 3, 6), []);
  assert.deepEqual(vscode.languages.getDiagnostics(uri), [], 'Syntax context must not retain semantic errors');
  await closeReverting(document);
  document = await openEditorDocument(uri);
  assert.deepEqual(await hover(uri, 3, 6), [], 'Closing/reopening must clear explicit context');
  await closeReverting(document);
  await checkSymlinkDotContext(workspace, root);
}

module.exports = { run };
