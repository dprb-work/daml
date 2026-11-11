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
assert.ok(vsix && isAbsolute(vsix) && lsp && isAbsolute(lsp), 'Set absolute DAML_VSIX_PATH and DAML_LSP_PATH');
const supported = ['configuration-completion', 'stale-context'];
const selected = process.env.DAML_ROOT_CONSUMER_CASE;
assert.ok(!selected || supported.includes(selected));
const scratch = await mkdtemp(join(tmpdir(), 'daml-root-consumer-'));
const executable = process.env.DAML_VSCODE_EXECUTABLE ?? await downloadAndUnzipVSCode('1.105.1');
const extensions = join(scratch, 'extensions');
const noSandbox = process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
const owned = [];
async function records(file) { return (await readFile(file, 'utf8')).split('\n').slice(0, -1).filter(Boolean).map(line => JSON.parse(line)); }
try {
  await mkdir(extensions);
  const cli = resolveCliArgsFromVSCodeExecutablePath(executable, { reuseMachineInstall: true });
  const install = spawnSync(cli[0], [...cli.slice(1), '--user-data-dir', join(scratch, 'installer'), '--extensions-dir', extensions,
    '--install-extension', vsix, '--force', ...noSandbox], { encoding: 'utf8', env: process.env });
  assert.equal(install.status, 0, install.stderr);
  for (const scenario of selected ? [selected] : supported) {
    const root = join(scratch, scenario);
    const workspace = join(root, 'workspace');
    const user = join(root, 'user');
    const status = join(root, 'wire.jsonl');
    const control = join(root, 'control');
    const original = join(root, 'original.cjs');
    const replacement = join(root, 'replacement.cjs');
    await mkdir(join(workspace, 'bundle'), { recursive: true });
    await mkdir(join(user, 'User'), { recursive: true });
    await writeFile(status, '');
    await writeFile(control, '');
    await writeFile(join(workspace, 'bundle', 'daml.bundle'), '{}\n');
    await writeFile(join(workspace, 'bundle', 'Config.daml'), 'closed !Object { endpoint: !String }\n');
    await writeFile(join(workspace, 'probe.daml'), '{ endpoint: "ok" }\n');
    // VS Code may resolve a shell PATH without Node; use the harness runtime.
    const launch = mode => `#!${process.execPath}\nprocess.env.DAML_ROOT_PROXY_MODE = ${JSON.stringify(mode)};\nrequire(${JSON.stringify(join(here, 'root-consumer-proxy.cjs'))});\n`;
    await writeFile(original, launch(scenario === 'stale-context' ? 'held-context' : 'plain'));
    await writeFile(replacement, launch('plain'));
    await chmod(original, 0o755);
    await chmod(replacement, 0o755);
    owned.push({ status, original, replacement });
    await writeFile(join(user, 'User', 'settings.json'), JSON.stringify({ 'definitivelyAssertedDaml.serverPath': original, 'security.workspace.trust.enabled': true }));
    try {
      await runTests({ vscodeExecutablePath: executable, extensionDevelopmentPath: join(here, 'driver'),
        extensionTestsPath: join(here, 'root-consumer.test.cjs'),
        extensionTestsEnv: { ...process.env, DAML_ROOT_CONSUMER_CASE: scenario, DAML_LSP_STATUS_FILE: status,
          DAML_ROOT_CONTROL: control, DAML_ROOT_ORIGINAL: original, DAML_ROOT_REPLACEMENT: replacement,
          DAML_ROOT_RELEASE_BEACON: `DAML held-context reader barrier ${scenario}` },
        launchArgs: [workspace, '--user-data-dir', user, '--extensions-dir', extensions, '--disable-workspace-trust', ...noSandbox],
      });
    } catch (error) { console.error(JSON.stringify({ event: 'root-consumer-failure', scenario, protocol: await records(status) }, null, 2)); throw error; }
  }
} finally {
  try {
    for (const entry of owned) for (const event of await records(entry.status)) {
      if (!['wrapper-start', 'child-spawn'].includes(event.event) || !Number.isSafeInteger(event.pid) || event.pid < 2) continue;
      let command;
      try { command = await readFile(`/proc/${event.pid}/cmdline`, 'utf8'); }
      catch (error) { if (error.code === 'ENOENT') continue; throw error; }
      const matches = event.event === 'child-spawn' ? command.includes(lsp) : command.includes(entry.original) || command.includes(entry.replacement);
      if (!matches) continue;
      try { process.kill(event.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
  } finally { await rm(scratch, { recursive: true, force: true }); }
}
