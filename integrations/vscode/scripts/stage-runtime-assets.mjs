import { chmod, copyFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extensionDirectory = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const clientDirectory = dirname(createRequire(import.meta.url).resolve('vscode-languageclient/package.json'));
const destination = resolve(extensionDirectory, 'dist/terminateProcess.sh');
await copyFile(resolve(clientDirectory, 'lib/node/terminateProcess.sh'), destination);
await chmod(destination, 0o755);
