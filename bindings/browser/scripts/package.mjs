import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { cargo, files, fingerprint, repository, root, run } from './build.mjs';

const provenance = JSON.parse(await readFile(join(root, 'BUILD.json'), 'utf8'));
if (provenance.profile !== 'release') throw new Error('package requires a release SDK build, not --dev');
if (provenance.source_sha256 !== await fingerprint()) throw new Error('SDK producer inputs changed after build');
const actualFiles = (await files(join(root, 'dist'))).map(path => relative(root, path).split('\\').join('/')).sort();
const recordedFiles = Object.keys(provenance.outputs).sort();
if (actualFiles.length !== recordedFiles.length || actualFiles.some((name, index) => name !== recordedFiles[index])) {
  throw new Error('built SDK distribution file set changed');
}
for (const [name, expected] of Object.entries(provenance.outputs)) {
  const actual = createHash('sha256').update(await readFile(join(root, name))).digest('hex');
  if (actual !== expected) throw new Error(`built SDK output changed: ${name}`);
}
if (run(cargo, ['about', '--version']).trim() !== 'cargo-about 0.9.2') throw new Error('packaging requires cargo-about 0.9.2');
const temporary = join(root, 'artifacts', 'license-config.toml');
await mkdir(join(root, 'artifacts'), { recursive: true });
const python = process.env.PYTHON ?? 'python3';
const config = run(python, ['-B', '-c', [
  'import importlib.util, sys',
  'spec = importlib.util.spec_from_file_location("daml_native_packaging", sys.argv[1])',
  'module = importlib.util.module_from_spec(spec)',
  'spec.loader.exec_module(module)',
  'print(module.make_about_config().replace("${HOST}", "wasm32-unknown-unknown"), end="")',
].join('\n'), join(repository, 'scripts', 'package-binaries.py')]);
try {
  await writeFile(temporary, config);
  const notices = run(cargo, ['about', 'generate', '--manifest-path', join(root, 'Cargo.toml'), '--config', temporary, '--locked', '--target', 'wasm32-unknown-unknown', '--fail', join(repository, 'scripts', 'third-party-notices.hbs')]);
  await writeFile(join(root, 'THIRD-PARTY-NOTICES.txt'), notices);
} finally {
  await rm(temporary, { force: true });
}
const packageInputs = {};
for (const name of ['LICENSE', 'SOURCES.md', 'README.md', 'THIRD-PARTY-NOTICES.txt']) {
  packageInputs[name] = createHash('sha256').update(await readFile(join(root, name))).digest('hex');
}
await writeFile(join(root, 'BUILD.json'), `${JSON.stringify({ ...provenance, package_inputs: packageInputs }, null, 2)}\n`);
const result = JSON.parse(run('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', join(root, 'artifacts')]))[0];
const archive = join(root, 'artifacts', result.filename);
const digest = createHash('sha256').update(await readFile(archive)).digest('hex');
await writeFile(`${archive}.sha256`, `${digest}  ${result.filename}\n`);
console.log(JSON.stringify({ archive, sha256: digest, git_commit: provenance.git_commit, git_dirty: provenance.git_dirty }, null, 2));
