import { spawn, spawnSync } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath, runTests } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const vsix = resolve(repo, 'integrations/vscode/definitively-asserted-daml-0.1.0.vsix');
const lsp = process.env.DAML_LSP_PATH;
if (!lsp || !lsp.startsWith('/')) throw new Error('Set DAML_LSP_PATH to the absolute path of the released daml-lsp executable.');
await readFile(vsix);
const executable = await downloadAndUnzipVSCode('1.105.1');
const scratch = await mkdtemp(join(tmpdir(), 'daml-vscode-verify-'));
const extensions = join(scratch, 'extensions');
const trustedWorkspace = join(scratch, 'trusted-workspace');
const untrustedWorkspace = join(scratch, 'untrusted-workspace');
const trustedUserData = join(scratch, 'trusted-user-data');
const untrustedUserData = join(scratch, 'untrusted-user-data');
const installerUserData = join(scratch, 'installer-user-data');
const workspaceOverride = join(scratch, 'workspace-server-sentinel');
const trustedLog = join(scratch, 'trusted-server-sentinel.log');
const untrustedLog = join(scratch, 'untrusted-server-sentinel.log');
const lifecycleWrapper = join(scratch, 'lifecycle-server-wrapper.cjs');
const lifecycleWrapperPid = join(scratch, 'lifecycle-wrapper.pid');
const lifecycleChildPid = join(scratch, 'lifecycle-server.pid');
const lifecycleStatusFile = join(scratch, 'lifecycle-status.jsonl');
const lifecycleStageFile = join(scratch, 'lifecycle-test-stages.jsonl');
const testEnv = { ...process.env, DAML_LSP_PATH: lsp, PATH: `${dirname(lsp)}:${process.env.PATH ?? ''}` };
await Promise.all([
  mkdir(extensions), mkdir(installerUserData, { recursive: true }),
  mkdir(trustedWorkspace, { recursive: true }), mkdir(join(trustedWorkspace, '.vscode'), { recursive: true }),
  mkdir(untrustedWorkspace, { recursive: true }),
  mkdir(join(trustedUserData, 'User'), { recursive: true }),
  mkdir(join(untrustedUserData, 'User'), { recursive: true }),
]);
await writeFile(workspaceOverride, [
  '#!/bin/sh',
  'printf "sentinel invoked\\n" >> "$DAML_SERVER_SENTINEL_LOG"',
  'exec "$DAML_SERVER_REAL_PATH" "$@"',
  '',
].join('\n'), 'utf8');
await chmod(workspaceOverride, 0o755);
await writeFile(lifecycleWrapper, `#!/usr/bin/env node
const { spawn } = require('node:child_process');
const { appendFileSync, writeFileSync } = require('node:fs');
const record = event => appendFileSync(${JSON.stringify(lifecycleStatusFile)}, JSON.stringify(event) + '\\n');
record({ event: 'wrapper-start', pid: process.pid, execPath: process.execPath });
writeFileSync(${JSON.stringify(lifecycleWrapperPid)}, String(process.pid));
let child;
try {
  child = spawn(${JSON.stringify(lsp)}, process.argv.slice(2), { stdio: 'inherit' });
} catch (error) {
  record({ event: 'spawn-error', code: error.code ?? null, message: error.message });
  process.exit(1);
}
record({ event: 'child-spawn', pid: child.pid ?? null, execPath: ${JSON.stringify(lsp)} });
if (child.pid !== undefined) writeFileSync(${JSON.stringify(lifecycleChildPid)}, String(child.pid));
child.once('error', error => {
  record({ event: 'spawn-error', code: error.code ?? null, message: error.message });
  process.exit(1);
});
child.once('exit', (code, signal) => {
  record({ event: 'child-exit', code, signal });
  if (code === 0) setInterval(() => {}, 60_000);
  else process.exit(code ?? 1);
});
`, 'utf8');
await chmod(lifecycleWrapper, 0o755);
await writeFile(join(trustedWorkspace, '.vscode', 'settings.json'), JSON.stringify({
  'definitivelyAssertedDaml.serverPath': workspaceOverride,
}, null, 2));
await Promise.all([
  writeFile(join(trustedUserData, 'User', 'settings.json'), JSON.stringify({
    'definitivelyAssertedDaml.serverPath': lifecycleWrapper,
    'security.workspace.trust.enabled': true,
  }, null, 2)),
  writeFile(join(untrustedUserData, 'User', 'settings.json'), JSON.stringify({
    'definitivelyAssertedDaml.serverPath': workspaceOverride,
    'security.workspace.trust.enabled': true,
  }, null, 2)),
]);
async function killOwnedProcess(pidPath, expectedCommand) {
  let pid;
  try {
    pid = Number(await readFile(pidPath, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return;
    throw error;
  }
  if (!Number.isSafeInteger(pid) || pid < 2) throw new Error(`Invalid process ID in ${pidPath}`);
  let commandLine;
  try {
    commandLine = await readFile(`/proc/${pid}/cmdline`, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return;
    throw error;
  }
  if (!commandLine.includes(expectedCommand)) return;
  try {
    process.kill(pid, 'SIGKILL');
  } catch (error) {
    if (error.code !== 'ESRCH') throw error;
  }
}
async function pidFileState(path) {
  try {
    return { path, value: (await readFile(path, 'utf8')).trim() };
  } catch (error) {
    return { path, error: { code: error.code ?? null, message: error.message } };
  }
}


let runSucceeded = false;
try {
  const noSandbox = process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [];
  const cliArgs = resolveCliArgsFromVSCodeExecutablePath(executable, { reuseMachineInstall: true });
  const installed = spawnSync(cliArgs[0], [
    ...cliArgs.slice(1), '--user-data-dir', installerUserData,
    '--extensions-dir', extensions, '--install-extension', vsix, '--force', ...noSandbox,
  ], { encoding: 'utf8', env: testEnv });
  if (installed.status !== 0) throw new Error(`VS Code failed to install the VSIX (${installed.status}): ${installed.stderr}`);

  const driver = join(here, 'driver');
  const testPath = join(here, 'extension.test.cjs');
  async function assertNoInvocation(logPath, scenario) {
    try {
      await readFile(logPath);
      throw new Error(`The ${scenario} serverPath sentinel was invoked`);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  async function runTrusted() {
    const scenarioEnv = {
      ...testEnv, DAML_VSCODE_SCENARIO: 'trusted',
      DAML_SERVER_SENTINEL_LOG: trustedLog, DAML_SERVER_REAL_PATH: lsp,
      DAML_LSP_WRAPPER_PID_FILE: lifecycleWrapperPid,
      DAML_LSP_CHILD_PID_FILE: lifecycleChildPid,
      DAML_LSP_STAGE_FILE: lifecycleStageFile,
    };
    await runTests({
      vscodeExecutablePath: executable,
      extensionDevelopmentPath: driver,
      extensionTestsPath: testPath,
      extensionTestsEnv: scenarioEnv,
      launchArgs: [trustedWorkspace, '--user-data-dir', trustedUserData, '--extensions-dir', extensions, '--disable-workspace-trust'],
    });
    await assertNoInvocation(trustedLog, 'trusted workspace override');
  }
  async function runUntrusted() {
    const scenarioEnv = {
      ...testEnv, DAML_VSCODE_SCENARIO: 'untrusted',
      DAML_SERVER_SENTINEL_LOG: untrustedLog, DAML_SERVER_REAL_PATH: lsp,
    };
    const child = spawn(executable, [
      untrustedWorkspace, '--user-data-dir', untrustedUserData, '--extensions-dir', extensions,
      '--extensionDevelopmentPath', driver, '--extensionTestsPath', testPath, ...noSandbox,
    ], { env: scenarioEnv, stdio: 'inherit' });
    const exitCode = await new Promise((resolveExit, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => {
        if (code === 0) resolveExit(0);
        else reject(new Error(`Untrusted VS Code extension host exited with ${code ?? signal}`));
      });
    });
    if (exitCode !== 0) throw new Error(`Untrusted VS Code extension host exited with ${exitCode}`);
    await assertNoInvocation(untrustedLog, 'untrusted user configuration');
  }

  await runTrusted();
  await runUntrusted();
  runSucceeded = true;
} finally {
  if (!runSucceeded) {
    async function readEvents(path) {
      try {
        const contents = await readFile(path, 'utf8');
        return { events: contents.split('\n').filter(Boolean).map(line => JSON.parse(line)) };
      } catch (error) {
        return { events: [], error: { code: error.code ?? null, message: error.message } };
      }
    }
    const [wrapperStatus, testStages, wrapperPid, childPid] = await Promise.all([
      readEvents(lifecycleStatusFile),
      readEvents(lifecycleStageFile),
      pidFileState(lifecycleWrapperPid),
      pidFileState(lifecycleChildPid),
    ]);
    console.error(JSON.stringify({
      event: 'lifecycle-wrapper-status',
      wrapperEvents: wrapperStatus.events,
      wrapperStatusError: wrapperStatus.error,
      testStages: testStages.events,
      testStagesError: testStages.error,
      pidFiles: { wrapper: wrapperPid, child: childPid },
    }, null, 2));
  }
  try {
    await killOwnedProcess(lifecycleChildPid, lsp);
    await killOwnedProcess(lifecycleWrapperPid, lifecycleWrapper);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
