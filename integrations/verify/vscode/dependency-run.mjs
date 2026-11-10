import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { downloadAndUnzipVSCode, runTests } from '@vscode/test-electron';

const here = dirname(fileURLToPath(import.meta.url));
const scratch = await mkdtemp(join(tmpdir(), 'daml-dependency-lifecycle-'));
try {
  const executable = await downloadAndUnzipVSCode('1.105.1');
  await runTests({
    vscodeExecutablePath: executable,
    extensionDevelopmentPath: join(here, 'driver'),
    extensionTestsPath: join(here, 'dependency-host.test.cjs'),
    launchArgs: [
      '--user-data-dir', join(scratch, 'user'), '--extensions-dir', join(scratch, 'extensions'),
      '--disable-workspace-trust',
      ...(process.env.DAML_VSCODE_NO_SANDBOX === '1' ? ['--no-sandbox'] : []),
    ],
  });
} finally {
  await rm(scratch, { recursive: true, force: true });
}
