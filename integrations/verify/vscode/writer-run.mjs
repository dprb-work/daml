import { spawnSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const here = dirname(fileURLToPath(import.meta.url));
const dependencyRequire = createRequire(join(here, '../../vscode/package.json'));
const { build } = dependencyRequire('esbuild');
const scratch = await mkdtemp(join(tmpdir(), 'daml-writer-lifecycle-'));
try {
  const compiled = join(scratch, 'session-writer.cjs');
  // Compile the actual production owner. Before this new owner exists, a
  // missing-module setup failure is not behavioral red.
  await build({
    entryPoints: [join(here, '../../vscode/src/session-writer.ts')],
    outfile: compiled, bundle: true, platform: 'node', format: 'cjs',
    // Share the installed dependency instances with the protocol test peer;
    // bundling duplicate ResponseError classes would create a false boundary.
    plugins: [{
      name: 'installed-dependency-boundaries',
      setup(builder) {
        builder.onResolve({ filter: /^[^./]/ }, args => ({
          path: dependencyRequire.resolve(args.path), external: true,
        }));
      },
    }],
  });
  const result = spawnSync(process.execPath, ['--test', join(here, 'writer.test.cjs')], {
    stdio: 'inherit', env: { ...process.env, DAML_WRITER_MODULE: compiled },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Writer lifecycle checks failed (${result.status ?? result.signal})`);
} finally { await rm(scratch, { recursive: true, force: true }); }
