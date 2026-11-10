const { spawn } = require('node:child_process');
const fs = require('node:fs');
const frames = require('./lsp-frames.cjs');
const record = event => fs.appendFileSync(process.env.DAML_LSP_STATUS_FILE, `${JSON.stringify({ session: process.pid, ...event })}\n`);
function encode(message) { const body = Buffer.from(JSON.stringify(message)); return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]); }
const holdContext = process.env.DAML_ROOT_PROXY_MODE === 'held-context';
const child = spawn(process.env.DAML_LSP_PATH, [], { stdio: ['pipe', 'pipe', 'inherit'] });
record({ event: 'wrapper-start', pid: process.pid, executable: process.argv[1] });
record({ event: 'child-spawn', pid: child.pid });
let contextId;
let contextUri;
let shutdown;
const held = [];
process.stdin.on('data', frames((message, frame) => {
  record({ event: 'outbound', message });
  if (holdContext && contextId === undefined && message.method === 'workspace/executeCommand' && message.params?.command === 'daml.setContext') {
    contextId = message.id;
    contextUri = message.params.arguments[0].uri;
  }
  if (holdContext && message.method === 'shutdown') {
    shutdown = frame;
    record({ event: 'shutdown-held', id: message.id });
    return;
  }
  if (!child.stdin.destroyed) child.stdin.write(frame);
}));
process.stdin.on('end', () => child.stdin.end());
child.stdout.on('data', frames((message, frame) => {
  if (holdContext && contextId !== undefined && message.id === contextId && message.method === undefined) {
    held.push(frame);
    record({ event: 'context-response-held', message });
    return;
  }
  if (holdContext && contextUri && message.method === 'daml/dependencies' && message.params.uri === contextUri) {
    held.push(frame);
    record({ event: 'dependencies-held', message });
    return;
  }
  process.stdout.write(frame);
}));
if (holdContext) fs.watch(process.env.DAML_ROOT_CONTROL, () => {
  const command = fs.readFileSync(process.env.DAML_ROOT_CONTROL, 'utf8');
  if (command === 'release-context' && held.length) {
    const pending = held.splice(0);
    for (const frame of pending) process.stdout.write(frame);
    // Public logging is an old-reader FIFO beacon, not an application response.
    process.stdout.write(encode({ jsonrpc: '2.0', method: 'window/logMessage', params: { type: 3, message: process.env.DAML_ROOT_RELEASE_BEACON } }));
    record({ event: 'context-released', frames: pending.length });
  }
  if (command === 'release-shutdown' && shutdown) {
    child.stdin.write(shutdown);
    shutdown = undefined;
    record({ event: 'shutdown-forwarded' });
  }
});
child.stdin.on('error', error => { if (!['EPIPE', 'ERR_STREAM_DESTROYED'].includes(error.code)) throw error; });
child.once('error', error => { record({ event: 'spawn-error', message: error.message }); process.exit(1); });
child.once('exit', (code, signal) => { record({ event: 'child-exit', code, signal }); process.exit(code ?? 1); });
