import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(await readFile(join(root, 'patches/client-lifecycle.json'), 'utf8'));
if (manifest.schemaVersion !== 1) throw new Error('Unsupported client correction manifest.');

function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

for (const [name, version] of Object.entries(manifest.packages)) {
  const installed = JSON.parse(await readFile(join(root, 'node_modules', name, 'package.json'), 'utf8'));
  if (installed.name !== name || installed.version !== version) {
    throw new Error(`Client preparation requires ${name}@${version}; found ${installed.name}@${installed.version}. Restore the locked graph with npm ci --ignore-scripts.`);
  }
}

// Admit the entire correction set before writing anything. Original and exactly
// corrected bytes are the only accepted inputs; interrupted preparation is safe
// to repeat, while local edits or a dependency update fail closed.
const outputs = [];
for (const file of manifest.files) {
  const path = join(root, 'node_modules', file.package, file.file);
  const original = await readFile(path);
  const digest = sha256(original);
  if (digest === file.correctedSha256) continue;
  if (digest !== file.originalSha256) {
    throw new Error(`Client correction source drift: ${file.package}/${file.file} (${digest}). Restore npm ci --ignore-scripts; do not patch node_modules by hand.`);
  }
  const lines = original.toString('utf8').match(/[^\n]*\n|[^\n]+$/g) ?? [];
  for (const edit of [...file.edits].reverse()) {
    const offset = edit.line - 1;
    if (lines.slice(offset, offset + edit.before.length).join('') !== edit.before.join('')) {
      throw new Error(`Invalid client correction payload: ${file.package}/${file.file}:${edit.line}.`);
    }
    lines.splice(offset, edit.before.length, ...edit.after);
  }
  const corrected = lines.join('');
  if (sha256(corrected) !== file.correctedSha256) {
    throw new Error(`Client correction output mismatch: ${file.package}/${file.file}.`);
  }
  outputs.push({ path, corrected });
}
for (const { path, corrected } of outputs) {
  await writeFile(path, corrected);
}
console.log(`Prepared pinned client lifecycle corrections (${outputs.length} files updated).`);
