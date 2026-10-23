import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const soulRoot = process.env.SOUL_REPO_PATH;
const damlLsp = process.env.DAML_LSP_PATH;
if (!soulRoot?.startsWith('/') || !damlLsp?.startsWith('/')) throw new Error('Set SOUL_REPO_PATH and DAML_LSP_PATH to absolute paths.');
const importSoul = path => import(pathToFileURL(resolve(soulRoot, path)).href);
const [{ Context }, { LocalFileSystem }, { SystemPrompt }, { ToolRuntime }, { apply: applyHost }, { LspTools }] = await Promise.all([
  importSoul('runtime/dsh/node_modules/@deepseek-ai/cordis/lib/index.js'),
  importSoul('runtime/dsh/node_modules/@deepseek-ai/dsh-fs-local/lib/index.js'),
  importSoul('runtime/dsh/node_modules/@deepseek-ai/dsh-system-prompt/lib/index.js'),
  importSoul('runtime/dsh/node_modules/@deepseek-ai/dsh-tools/lib/index.js'),
  importSoul('runtime/dsh/extensions/src/host.ts'),
  importSoul('runtime/dsh/extensions/src/lsp.ts'),
]);

const root = await mkdtemp(join(tmpdir(), 'dsh-daml-native-host-'));
const sourcePath = join(root, 'trial.daml');
const canonicalPath = join(root, 'canonical.daml');
const invalidSource = '"😀é" ?';
const validSource = '{message:"😀é"}';
const canonicalSource = '{ message: "😀é" }\n';
const signal = new AbortController().signal;
let ctx;
let lsp;
try {
  await Promise.all([writeFile(sourcePath, invalidSource), writeFile(canonicalPath, canonicalSource)]);
  ctx = new Context();
  await ctx.plugin(LocalFileSystem, { cwd: root });
  await ctx.plugin(SystemPrompt);
  await ctx.plugin(ToolRuntime, { mode: 'native' });
  applyHost(ctx, { workspaceRoot: root });
  lsp = new LspTools(ctx, { servers: [{ language: 'daml', command: damlLsp }] });

  async function call(name, args, callId) {
    const result = await ctx.tools.execute({ name, arguments: args, callId, signal });
    if (result.isError) throw new Error(`${name} failed: ${JSON.stringify(result.error)}`);
    return result.value;
  }

  const invalid = await call('lsp_diagnostics', { language: 'daml', path: 'trial.daml' }, 'daml-diagnostics-invalid');
  assert.equal(invalid.diagnostics.length, 1);
  assert.deepEqual(invalid.diagnostics[0].range, { start: { line: 0, character: 6 }, end: { line: 0, character: 7 } });

  await writeFile(sourcePath, validSource);
  const cleared = await call('lsp_diagnostics', { language: 'daml', path: 'trial.daml' }, 'daml-diagnostics-cleared');
  assert.deepEqual(cleared.diagnostics, []);

  const before = await ctx.fs.readText(await ctx.fs.resolve(sourcePath));
  const preview = await call('lsp_format', { language: 'daml', path: 'trial.daml', tabSize: 2, insertSpaces: true }, 'daml-format-preview');
  assert.equal(preview.uri, `file://${sourcePath}`);
  assert.equal(typeof preview.version, 'string');
  assert.equal(preview.edits.length, 1);
  assert.deepEqual(preview.edits[0].range, { start: { line: 0, character: 0 }, end: { line: 0, character: 15 } });
  assert.equal(preview.edits[0].newText, canonicalSource);
  assert.equal(await ctx.fs.readText(await ctx.fs.resolve(sourcePath)), before, 'format preview must not write through the native DSH filesystem');

  const unchanged = await call('lsp_format', { language: 'daml', path: 'canonical.daml', tabSize: 2, insertSpaces: true }, 'daml-format-idempotent');
  assert.deepEqual(unchanged.edits, []);
  console.log(JSON.stringify({ passed: true, scope: 'Cordis ToolRuntime + DSH LocalFileSystem + canonical NativeWorkspaceAccess + released daml-lsp', invalid, cleared, preview, unchanged }, null, 2));
} finally {
  try {
    if (lsp) await lsp.dispose();
  } finally {
    try {
      if (ctx) await ctx.fiber.dispose();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
}
