import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath, runTests } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const vsix = process.env.DAML_VSIX_PATH;
const lsp = process.env.DAML_LSP_PATH;
assert.ok(vsix && isAbsolute(vsix), 'Set absolute DAML_VSIX_PATH');
assert.ok(lsp && isAbsolute(lsp), 'Set absolute DAML_LSP_PATH');
const selected = process.env.DAML_VSCODE_ADVERSARIAL_SCENARIO;
const supported = [
  'startup-timeout', 'startup-replacement', 'startup-deactivation', 'crash-lineage',
  'startup-coalescing', 'retirement-failure',
];
const scenarios = selected ? [selected] : supported;
assert.ok(scenarios.every(scenario => supported.includes(scenario)));
const executable = process.env.DAML_VSCODE_EXECUTABLE ?? await downloadAndUnzipVSCode('1.105.1');
const scratch = await mkdtemp(join(tmpdir(), 'daml-adversarial-lifecycle-'));
const extensions = join(scratch, 'extensions');
const noSandbox = process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
const statuses = [];
async function events(file) {
  try { return (await readFile(file, 'utf8')).split('\n').slice(0, -1).filter(Boolean).map(line => JSON.parse(line)); }
  catch (error) { if (error.code === 'ENOENT') return []; throw error; }
}
try {
  await mkdir(extensions);
  const cli = resolveCliArgsFromVSCodeExecutablePath(executable, { reuseMachineInstall: true });
  const install = spawnSync(cli[0], [
    ...cli.slice(1), '--user-data-dir', join(scratch, 'installer'), '--extensions-dir', extensions,
    '--install-extension', vsix, '--force', ...noSandbox,
  ], { encoding: 'utf8', env: process.env });
  assert.equal(install.status, 0, `VSIX installation failed: ${install.stderr}`);
  for (const scenario of scenarios) {
    const root = join(scratch, scenario);
    const workspace = join(root, 'workspace');
    const user = join(root, 'user');
    const status = join(root, 'protocol.jsonl');
    const control = join(root, 'crash-control');
    const wrapper = join(root, 'server-wrapper.cjs');
    const alternate = join(root, 'alternate-wrapper.cjs');
    const finalWrapper = join(root, 'final-wrapper.cjs');
    statuses.push({ scenario, file: status, wrapper, alternate, finalWrapper });
    await mkdir(workspace, { recursive: true });
    await mkdir(join(user, 'User'), { recursive: true });
    await writeFile(status, '');
    await writeFile(control, '0');
    const launch = withhold => `#!/usr/bin/env node\nprocess.env.DAML_LSP_WITHHOLD_INITIALIZE = ${JSON.stringify(withhold ? '1' : '0')};\nrequire(${JSON.stringify(join(here, 'adversarial-proxy.cjs'))});\n`;
    await writeFile(wrapper, launch(scenario.startsWith('startup-')));
    await writeFile(alternate, launch(scenario === 'startup-coalescing'));
    await writeFile(finalWrapper, launch(false));
    await chmod(wrapper, 0o755);
    await chmod(alternate, 0o755);
    await chmod(finalWrapper, 0o755);
    await writeFile(join(user, 'User', 'settings.json'), JSON.stringify({
      'definitivelyAssertedDaml.serverPath': wrapper,
      'security.workspace.trust.enabled': true,
    }));
    try {
      await runTests({
        vscodeExecutablePath: executable,
        extensionDevelopmentPath: join(here, 'driver'),
        extensionTestsPath: join(here, 'adversarial.test.cjs'),
        extensionTestsEnv: {
          ...process.env, DAML_VSCODE_SCENARIO: scenario, DAML_LSP_STATUS_FILE: status,
          DAML_LSP_CRASH_CONTROL: control, DAML_LSP_ALTERNATE_PATH: alternate,
          DAML_LSP_FINAL_PATH: finalWrapper,
        },
        launchArgs: [workspace, '--user-data-dir', user, '--extensions-dir', extensions, '--disable-workspace-trust', ...noSandbox],
      });
    } catch (error) {
      console.error(JSON.stringify({ event: 'adversarial-lifecycle-failure', scenario, protocol: await events(status) }, null, 2));
      throw error;
    }
  }
} finally {
  try {
    for (const status of statuses) {
      for (const event of await events(status.file)) {
        if (event.event !== 'wrapper-start' && event.event !== 'child-spawn') continue;
        const pid = event.pid;
        if (!Number.isSafeInteger(pid) || pid < 2) continue;
        let command;
        try { command = await readFile(`/proc/${pid}/cmdline`, 'utf8'); }
        catch (error) { if (error.code === 'ENOENT') continue; throw error; }
        const owned = event.event === 'child-spawn' ? command.includes(lsp)
          : command.includes(status.wrapper) || command.includes(status.alternate) || command.includes(status.finalWrapper);
        if (!owned) continue;
        try { process.kill(pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
    }
  } finally { await rm(scratch, { recursive: true, force: true }); }
}
