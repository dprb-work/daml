import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const vsix = resolve(repo, 'integrations/vscode/definitively-asserted-daml-0.1.0.vsix');
const lsp = process.env.DAML_LSP_PATH;
if (!lsp || !lsp.startsWith('/')) throw new Error('Set DAML_LSP_PATH to the absolute path of daml-lsp.');
const port = process.env.VSCODE_CDP_PORT ?? '9222';
await readFile(vsix);
const scratch = await mkdtemp(join(tmpdir(), 'daml-vscode-visual-'));
const userData = join(scratch, 'user-data');
const extensions = join(scratch, 'extensions');
const workspace = join(scratch, 'workspace');
await Promise.all([mkdir(userData), mkdir(extensions), mkdir(workspace)]);
const file = join(workspace, 'visual.daml');
await writeFile(file, '{ message: "😀é" }\n', 'utf8');
const testEnv = { ...process.env, DAML_LSP_PATH: lsp, PATH: `${dirname(lsp)}:${process.env.PATH ?? ''}` };
let editor;
try {
  const executable = await downloadAndUnzipVSCode('1.105.1');
  const noSandbox = process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
  const cliArgs = resolveCliArgsFromVSCodeExecutablePath(executable, { reuseMachineInstall: true });
  const installed = spawn(cliArgs[0], [
    ...cliArgs.slice(1), '--user-data-dir', userData, '--extensions-dir', extensions,
    '--install-extension', vsix, '--force', ...noSandbox,
  ], { env: testEnv, stdio: 'inherit' });
  const installedExit = await new Promise(resolveExit => installed.once('exit', resolveExit));
  if (installedExit !== 0) throw new Error(`VS Code failed to install the VSIX (${installedExit})`);
  editor = spawn(executable, [file, '--new-window', '--user-data-dir', userData, '--extensions-dir', extensions, '--disable-workspace-trust', `--remote-debugging-port=${port}`, ...noSandbox], { env: testEnv, stdio: 'inherit' });
  console.log(JSON.stringify({ pid: editor.pid, cdp: `http://127.0.0.1:${port}`, file }));
  await new Promise((resolveExit, rejectExit) => {
    editor.once('error', rejectExit);
    editor.once('exit', code => resolveExit(code));
    process.once('SIGINT', () => editor.kill('SIGTERM'));
    process.once('SIGTERM', () => editor.kill('SIGTERM'));
  });
} finally {
  if (editor && editor.exitCode === null) editor.kill('SIGTERM');
  await rm(scratch, { recursive: true, force: true });
}
