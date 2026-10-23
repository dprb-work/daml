import { copyFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const extensionDirectory = resolve(scriptDirectory, '..');
await copyFile(resolve(extensionDirectory, '../../LICENSE'), resolve(extensionDirectory, 'LICENSE'));
