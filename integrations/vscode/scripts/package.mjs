import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { access, appendFile, copyFile, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(root, '../..');
const manifest = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
const archive = join(root, `${manifest.name}-${manifest.version}.vsix`);
const sidecar = `${archive}.sha256`;

function run(command, args, capture = false) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    stdio: capture ? 'pipe' : 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed (${result.status}): ${result.stderr ?? ''}`);
  }
  return result.stdout;
}

for (const path of [archive, sidecar]) {
  try {
    await access(path);
  } catch (error) {
    if (error.code === 'ENOENT') continue;
    throw error;
  }
  throw new Error(`Candidate already exists; move it before packaging: ${path}`);
}

const npm = process.env.npm_execpath;
if (!npm) throw new Error('Run this producer through npm run package.');
const commit = run('git', ['rev-parse', 'HEAD'], true).trim();
const dirtyBefore = run('git', ['status', '--porcelain', '--untracked-files=normal'], true).trim() !== '';
run(process.execPath, [npm, 'run', 'build']);
run(process.execPath, [join(root, 'scripts/stage-license.mjs')]);
run(process.execPath, [join(root, 'scripts/stage-third-party-notices.mjs')]);
await copyFile(join(repository, 'SOURCES.md'), join(root, 'SOURCES.md'));
if (run('git', ['rev-parse', 'HEAD'], true).trim() !== commit) {
  throw new Error('Source revision changed during VSIX preparation.');
}
const provenance = {
  kind: 'vsix',
  version: manifest.version,
  git_commit: commit,
  git_dirty: dirtyBefore || run('git', ['status', '--porcelain', '--untracked-files=normal'], true).trim() !== '',
  node: process.version,
};
await writeFile(join(root, 'BUILD.json'), `${JSON.stringify(provenance, null, 2)}\n`);

// Invoke the pinned JS entrypoint directly: npm/.cmd shims are not portable
// subprocess executables across developer platforms.
const vsceRoot = join(root, 'node_modules/@vscode/vsce');
const vsce = JSON.parse(await readFile(join(vsceRoot, 'package.json'), 'utf8'));
run(process.execPath, [join(vsceRoot, vsce.bin.vsce), 'package', '--no-dependencies', '--out', archive]);
const digest = createHash('sha256').update(await readFile(archive)).digest('hex');
await writeFile(sidecar, `${digest}  ${manifest.name}-${manifest.version}.vsix\n`);
if (process.env.GITHUB_OUTPUT) {
  await appendFile(process.env.GITHUB_OUTPUT, `vsix=${archive}\nchecksum=${sidecar}\n`);
}
console.log(JSON.stringify({ archive, sha256: digest, ...provenance }, null, 2));
