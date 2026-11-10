// A transparent stdio observer around the real native server. It deliberately
// withholds one context response, not its request, to expose lifecycle barriers.
const { spawn } = require('node:child_process');
const { appendFileSync, writeFileSync } = require('node:fs');
const record = event => appendFileSync(process.env.DAML_LSP_STATUS_FILE,
  `${JSON.stringify({ session: process.pid, ...event })}\n`);
const linger = process.env.DAML_LSP_PROXY_LINGER === '1';
record({ event: 'wrapper-start', pid: process.pid, execPath: process.execPath, linger });
if (linger) writeFileSync(process.env.DAML_LSP_WRAPPER_PID_FILE, String(process.pid));
const child = spawn(process.env.DAML_SERVER_REAL_PATH, process.argv.slice(2), { stdio: ['pipe', 'pipe', 'inherit'] });
record({ event: 'child-spawn', pid: child.pid ?? null, execPath: process.env.DAML_SERVER_REAL_PATH });
if (linger && child.pid !== undefined) writeFileSync(process.env.DAML_LSP_CHILD_PID_FILE, String(child.pid));
let withheldId;
let shutdownId;
const frames = require('./lsp-frames.cjs');
process.stdin.on('data', frames((message, frame) => {
  // Capture before forwarding: a tolerant server cannot hide bad client order.
  record({ event: 'outbound', message });
  if (message.method === 'shutdown') shutdownId = message.id;
  if (linger && message.method === 'workspace/executeCommand'
      && message.params?.command === 'daml.setContext' && withheldId === undefined) {
    withheldId = message.id;
    record({ event: 'context-request', id: withheldId });
  }
  if (!child.stdin.destroyed) child.stdin.write(frame);
}));
process.stdin.on('end', () => child.stdin.end());
child.stdout.on('data', frames((message, frame) => {
  if (withheldId !== undefined && message.id === withheldId && message.method === undefined) {
    record({ event: 'context-response-withheld', id: withheldId });
    return;
  }
  if (shutdownId !== undefined && message.id === shutdownId && message.method === undefined) {
    record({ event: 'shutdown-response', message });
  }
  process.stdout.write(frame);
}));
// Exit may race a final input chunk; preserve capture without crashing on EPIPE.
child.stdin.on('error', error => {
  record({ event: 'child-stdin-error', code: error.code });
  if (error.code !== 'EPIPE' && error.code !== 'ERR_STREAM_DESTROYED') throw error;
});
child.once('error', error => {
  record({ event: 'spawn-error', code: error.code ?? null, message: error.message });
  process.exit(1);
});
child.once('exit', (code, signal) => {
  record({ event: 'child-exit', code, signal });
  if (linger && code === 0) setInterval(() => {}, 60_000);
  else process.exit(code ?? 1);
});
