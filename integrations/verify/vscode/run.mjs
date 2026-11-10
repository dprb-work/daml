import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath, runTests } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const vsix = process.env.DAML_VSIX_PATH;
if (!vsix || !isAbsolute(vsix)) throw new Error('Set DAML_VSIX_PATH to the absolute path of the packaged VSIX.');
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
const replacementWrapper = join(scratch, 'replacement-server-wrapper.cjs');
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
const proxy = join(here, 'lifecycle-proxy.cjs');
await writeFile(lifecycleWrapper, `#!/usr/bin/env node
process.env.DAML_LSP_PROXY_LINGER = '1';
require(${JSON.stringify(proxy)});
`, 'utf8');
await writeFile(replacementWrapper, `#!/usr/bin/env node
process.env.DAML_LSP_PROXY_LINGER = '0';
require(${JSON.stringify(proxy)});
`, 'utf8');
await chmod(replacementWrapper, 0o755);
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
      DAML_LSP_STATUS_FILE: lifecycleStatusFile,
      DAML_LSP_REPLACEMENT_PATH: replacementWrapper,
      DAML_LSP_WRAPPER_PATH: lifecycleWrapper,
    };
    await runTests({
      vscodeExecutablePath: executable,
      extensionDevelopmentPath: driver,
      extensionTestsPath: testPath,
      extensionTestsEnv: scenarioEnv,
      launchArgs: [trustedWorkspace, '--user-data-dir', trustedUserData, '--extensions-dir', extensions, '--disable-workspace-trust'],
    });
    await assertNoInvocation(trustedLog, 'trusted workspace override');
    const events = (await readFile(lifecycleStatusFile, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
    const sessions = events.filter(event => event.event === 'wrapper-start');
    assert.equal(sessions.length, 2, 'one original and one current replacement may start; deactivation must not reactivate');
    assert.ok(events.some(event => event.event === 'context-response-withheld'), 'the context operation must really be awaiting a missing response during replacement');
    for (const session of sessions) {
      const messages = events.filter(event => event.session === session.session && event.event === 'outbound').map(event => event.message);
      const shutdown = messages.findIndex(message => message.method === 'shutdown');
      assert.ok(shutdown >= 0, `session ${session.session} must submit shutdown`);
      assert.equal(messages.filter(message => message.method === 'shutdown').length, 1);
      assert.deepEqual(messages.slice(shutdown + 1).map(message => message.method), ['exit'],
        `session ${session.session} sent application traffic after shutdown`);
      const stream = events.filter(event => event.session === session.session);
      const acknowledgment = stream.findIndex(event => event.event === 'shutdown-response');
      const exit = stream.findIndex(event => event.event === 'outbound' && event.message.method === 'exit');
      assert.ok(acknowledgment >= 0 && acknowledgment < exit, 'exit requires the shutdown acknowledgment');
      assert.equal(stream[acknowledgment].message.id, messages[shutdown].id);
      assert.equal(stream[acknowledgment].message.result, null);
    }
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
    async function readLaunchLogs(userData) {
      const logs = [];
      async function visit(directory, depth) {
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
            await visit(path, depth + 1);
          } else if (entry.isFile() && (entry.name === 'exthost.log' || entry.name.toLowerCase().includes('daml'))) {
            try {
              const text = await readFile(path, 'utf8');
              logs.push({ path, omittedCharacters: Math.max(0, text.length - 65_536), text: text.slice(-65_536) });
            } catch (error) {
              logs.push({ path, error: error.message });
            }
          }
          if (logs.length >= 20) break;
        }
      }
      await visit(join(userData, 'logs'), 0);
      return logs;
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
    console.error(JSON.stringify({
      event: 'lifecycle-vscode-launch-logs',
      trusted: await readLaunchLogs(trustedUserData),
      untrusted: await readLaunchLogs(untrustedUserData),
    }, null, 2));
  }
  try {
    await killOwnedProcess(lifecycleChildPid, lsp);
    await killOwnedProcess(lifecycleWrapperPid, lifecycleWrapper);
    // Replacement normally exits through deactivation. Clean up a failed run's
    // remaining proxy/native child by recorded, task-owned PIDs as well.
    try {
      const events = (await readFile(lifecycleStatusFile, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
      for (const event of events) {
        if (event.event !== 'wrapper-start' && event.event !== 'child-spawn') continue;
        const pid = event.pid;
        if (!Number.isSafeInteger(pid) || pid < 2) continue;
        let command;
        try { command = await readFile(`/proc/${pid}/cmdline`, 'utf8'); }
        catch (error) { if (error.code === 'ENOENT') continue; throw error; }
        const expected = event.event === 'child-spawn' ? lsp : replacementWrapper;
        if (!command.includes(expected)) continue;
        try { process.kill(pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
