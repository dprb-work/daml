import { readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const extensionDirectory = resolve(scriptDirectory, '..');
const metadataPath = resolve(extensionDirectory, 'dist/meta.json');
const metadata = JSON.parse(await readFile(metadataPath, 'utf8'));
const packageDirectories = new Set();

for (const input of Object.keys(metadata.inputs)) {
  let directory = resolve(extensionDirectory, input);
  while (directory !== dirname(directory)) {
    try {
      const manifest = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
      if (manifest.name && manifest.version && directory.split(sep).includes('node_modules')) {
        packageDirectories.add(directory);
        break;
      }
    } catch {
      // Continue until the owning package manifest or filesystem root.
    }
    directory = dirname(directory);
  }
}

const packages = [];
for (const directory of [...packageDirectories].sort()) {
  const manifest = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
  const files = (await readdir(directory))
    .filter(file => /^(license|licence|copying|notice)(?:\.|$)/i.test(file))
    .sort();
  if (files.length === 0) {
    throw new Error(`No bundled license or notice file found for ${manifest.name}@${manifest.version}`);
  }
  const notices = [];
  for (const file of files) {
    notices.push(`--- ${manifest.name}@${manifest.version} / ${file} ---\n${(await readFile(join(directory, file), 'utf8')).trim()}`);
  }
  packages.push({ name: manifest.name, version: manifest.version, notices });
}

const output = [
  'Third-party notices for bundled runtime dependencies',
  'Generated from the esbuild metafile; do not edit by hand.',
  ...packages.flatMap(pkg => [`\n${pkg.name}@${pkg.version}`, ...pkg.notices]),
  ''
].join('\n');
await writeFile(resolve(extensionDirectory, 'THIRD-PARTY-NOTICES.txt'), output);
