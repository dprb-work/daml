import { createHash } from 'node:crypto';
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const repository = resolve(root, '../..');
export const cargo = process.env.CARGO ?? 'cargo';
const generator = process.env.WASM_BINDGEN ?? 'wasm-bindgen';

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed (${result.status}): ${result.stderr ?? ''}`);
  return result.stdout;
}

export async function files(directory) {
  const out = [];
  for (const item of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, item.name);
    if (item.isDirectory()) out.push(...await files(path));
    else if (item.isFile()) out.push(path);
    else throw new Error(`unsupported build-input entry: ${path}`);
  }
  return out;
}

export async function fingerprint() {
  const selected = [
    join(repository, 'Cargo.toml'), join(repository, 'rust-toolchain.toml'),
    join(repository, 'LICENSE'), join(repository, 'SOURCES.md'),
    join(repository, 'deny.toml'), join(repository, 'scripts/package-binaries.py'),
    join(repository, 'scripts/third-party-notices.hbs'),
    join(root, 'Cargo.toml'), join(root, 'Cargo.lock'), join(root, 'package.json'),
    ...await files(join(repository, 'src')), ...await files(join(root, 'src')),
    ...await files(join(root, 'scripts')),
  ].sort();
  const digest = createHash('sha256');
  for (const path of selected) {
    digest.update(relative(repository, path).split('\\').join('/'));
    digest.update('\0');
    digest.update(await readFile(path));
    digest.update('\0');
  }
  return digest.digest('hex');
}

export async function build(args = process.argv.slice(2)) {
  if (args.some(value => value !== '--dev') || args.length > 1) throw new Error('usage: build.mjs [--dev]');
  const profile = args.includes('--dev') ? 'debug' : 'release';
  const version = run(generator, ['--version']).trim();
  if (version !== 'wasm-bindgen 0.2.129') throw new Error(`expected wasm-bindgen 0.2.129, got ${version}`);
  const before = await fingerprint();
  const metadata = JSON.parse(run(cargo, ['metadata', '--locked', '--no-deps', '--format-version', '1', '--manifest-path', join(root, 'Cargo.toml')]));
  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const crate = metadata.packages.find(item => item.name === 'daml-browser');
  if (crate?.version !== pkg.version) throw new Error('Cargo and npm SDK versions differ');
  const command = ['build', '--locked', '--lib', '--manifest-path', join(root, 'Cargo.toml'), '--target', 'wasm32-unknown-unknown'];
  if (profile === 'release') command.push('--release');
  run(cargo, command, { stdio: 'inherit' });
  const dist = join(root, 'dist');
  await rm(dist, { recursive: true, force: true });
  await mkdir(dist, { recursive: true });
  run(generator, [join(metadata.target_directory, 'wasm32-unknown-unknown', profile, 'daml_browser.wasm'), '--target', 'web', '--out-dir', dist, '--out-name', 'daml_browser'], { stdio: 'inherit' });
  if (await fingerprint() !== before) throw new Error('producer inputs changed during SDK build; output is not a verified source snapshot');
  const outputs = {};
  for (const path of (await files(dist)).sort()) {
    outputs[relative(root, path).split('\\').join('/')] = createHash('sha256').update(await readFile(path)).digest('hex');
  }
  const provenance = {
    package: pkg.name, version: pkg.version, target: 'wasm32-unknown-unknown', profile,
    git_commit: run('git', ['rev-parse', 'HEAD'], { cwd: repository }).trim(),
    git_dirty: run('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: repository }).trim().length !== 0,
    source_sha256: before, cargo: run(cargo, ['--version']).trim(), generator: version,
    outputs,
  };
  await writeFile(join(root, 'BUILD.json'), `${JSON.stringify(provenance, null, 2)}\n`);
  await cp(join(repository, 'LICENSE'), join(root, 'LICENSE'));
  await cp(join(repository, 'SOURCES.md'), join(root, 'SOURCES.md'));
  console.log(JSON.stringify(provenance, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await build();
