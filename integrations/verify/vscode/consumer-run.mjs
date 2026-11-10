import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { downloadAndUnzipVSCode, runTests } from '@vscode/test-electron';
const here = dirname(fileURLToPath(import.meta.url));
const dependencyRequire = createRequire(join(here, '../../vscode/package.json'));
const { build } = dependencyRequire('esbuild');
const scratch = await mkdtemp(join(tmpdir(), 'daml-sdk-consumer-'));
try {
  const compiled = join(scratch, 'session-client.cjs');
  await build({
    entryPoints: [join(here, '../../vscode/src/session-client.ts')], outfile: compiled,
    bundle: true, platform: 'node', format: 'cjs', external: ['vscode'],
    plugins: [{ name: 'real-installed-dependencies', setup(builder) {
      builder.onResolve({ filter: /^[^./]/ }, args => ({
        path: args.path === 'vscode' ? args.path : dependencyRequire.resolve(args.path), external: true,
      }));
    } }],
  });
  const executable = process.env.DAML_VSCODE_EXECUTABLE ?? await downloadAndUnzipVSCode('1.105.1');
  await runTests({
    vscodeExecutablePath: executable, extensionDevelopmentPath: join(here, 'driver'),
    extensionTestsPath: join(here, 'consumer.test.cjs'),
    extensionTestsEnv: { ...process.env, DAML_SESSION_CLIENT_MODULE: compiled, DAML_CONSUMER_SCRATCH: scratch },
    launchArgs: ['--user-data-dir', join(scratch, 'user'), '--extensions-dir', join(scratch, 'extensions'),
      '--disable-workspace-trust', ...(process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : [])],
  });
} finally { await rm(scratch, { recursive: true, force: true }); }
