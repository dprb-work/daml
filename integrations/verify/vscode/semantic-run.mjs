import { spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import { access, cp, mkdir, mkdtemp, readdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath, runTests } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const vsix = resolve(repo, 'integrations/vscode/definitively-asserted-daml-0.1.0.vsix');
const lsp = process.env.DAML_LSP_PATH;
if (!lsp || !isAbsolute(lsp)) throw new Error('Set DAML_LSP_PATH to the absolute path of the source-built daml-lsp.');
await access(lsp, constants.X_OK);
await readFile(vsix); // Main builds the source VSIX; this runner never installs into the host profile.
const executable = await downloadAndUnzipVSCode('1.105.1');
const scratch = await mkdtemp(join(tmpdir(), 'daml-semantic-editor-'));
const fixture = join(scratch, 'fixture');
const extensions = join(scratch, 'extensions');
const userData = join(scratch, 'user-data');
const installerData = join(scratch, 'installer-data');
const noSandbox = process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
const verifyTrace = process.env.DAML_VERIFY_TRACE === '1';
const testEnv = { ...process.env, DAML_LSP_PATH: lsp, DAML_SEMANTIC_FIXTURE: fixture };
let runSucceeded = false;
try {
  await cp(join(repo, 'tests/fixtures/semantic-editor'), fixture, { recursive: true, errorOnExist: true });
  // Seed all leaf-link probes before any Code process starts. The measured tests
  // later share the providers directory backend proven by Http watch events;
  // queued setup writes must never masquerade as a missing link notification.
  for (const [name, prefix, chained] of [
    ['file-retarget', 'DirectRetarget', false],
    ['file-retarget-chain', 'ChainedRetarget', true],
  ]) {
    const bundle = join(fixture, 'providers');
    const stringName = `${prefix}StringDef.daml`;
    const intName = `${prefix}IntDef.daml`;
    const middleName = `${prefix}Middle.daml`;
    const aliasStem = `${prefix}Alias`;
    await Promise.all([
      writeFile(join(bundle, stringName), '!String\n', 'utf8'),
      writeFile(join(bundle, intName), '!Int\n', 'utf8'),
      writeFile(join(fixture, 'workspace', `${name}.daml`), `!self.${aliasStem} "kept"\n`, 'utf8'),
    ]);
    if (chained) await symlink(stringName, join(bundle, middleName), 'file');
    await symlink(chained ? middleName : stringName, join(bundle, `${aliasStem}.daml`), 'file');
  }
  await Promise.all([
    mkdir(extensions), mkdir(installerData), mkdir(join(userData, 'User'), { recursive: true }),
  ]);
  await writeFile(join(userData, 'User', 'settings.json'), JSON.stringify({
    'definitivelyAssertedDaml.serverPath': lsp,
    'security.workspace.trust.enabled': true,
    'files.autoSave': 'off',
    'workbench.editor.enablePreview': false,
    ...(verifyTrace ? { 'definitively-asserted-daml.trace.server': 'verbose' } : {}),
  }, null, 2));
  const cli = resolveCliArgsFromVSCodeExecutablePath(executable, { reuseMachineInstall: true });
  const installed = spawnSync(cli[0], [
    ...cli.slice(1), '--user-data-dir', installerData, '--extensions-dir', extensions,
    '--install-extension', vsix, '--force', ...noSandbox,
  ], { encoding: 'utf8', env: testEnv, timeout: 120_000 });
  if (installed.error || installed.status !== 0) {
    throw new Error(`Failed to install built VSIX into isolated profile: ${installed.error?.message ?? installed.stderr}`);
  }
  // test-electron owns and reaps the Electron process; the real extension owns its
  // language client/server. The existing lifecycle verifier separately checks reaping.
  await runTests({
    vscodeExecutablePath: executable,
    extensionDevelopmentPath: join(here, 'driver'),
    extensionTestsPath: join(here, 'semantic.test.cjs'),
    extensionTestsEnv: testEnv,
    launchArgs: [
      join(fixture, 'workspace'), '--user-data-dir', userData, '--extensions-dir', extensions,
      '--disable-workspace-trust', '--skip-welcome', '--skip-release-notes', ...noSandbox,
      ...(verifyTrace ? ['--log', 'trace'] : []),
    ],
  });
  runSucceeded = true;
} finally {
  if (!runSucceeded) {
    const logs = [];
    async function visitLogs(directory, depth) {
      if (depth > 8 || logs.length >= 20) return;
      let entries;
      try {
        entries = await readdir(directory, { withFileTypes: true });
      } catch (error) {
        if (error.code !== 'ENOENT') logs.push({ path: directory, error: error.message });
        return;
      }
      for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
          await visitLogs(path, depth + 1);
        } else if (entry.isFile() && (
          ['exthost.log', 'sharedprocess.log', 'renderer.log', 'window.log', 'main.log'].includes(entry.name)
          || entry.name.toLowerCase().includes('watch')
          || entry.name.toLowerCase().includes('daml')
        )) {
          try {
            const text = await readFile(path, 'utf8');
            const traceLog = entry.name.toLowerCase().includes('daml') || entry.name.toLowerCase().includes('watch');
            const limit = verifyTrace && traceLog ? 262_144 : 65_536;
            logs.push({ path, omittedCharacters: Math.max(0, text.length - limit), text: text.slice(-limit) });
          } catch (error) {
            logs.push({ path, error: error.message });
          }
        }
        if (logs.length >= 20) break;
      }
    }
    await visitLogs(join(userData, 'logs'), 0);
    console.error(JSON.stringify({ event: 'semantic-editor-failure', logs }, null, 2));
  }
  await rm(scratch, { recursive: true, force: true });
}
