const { spawn } = require('node:child_process');
const { appendFileSync, readFileSync, watch } = require('node:fs');
const frames = require('./lsp-frames.cjs');
const scenario = process.env.DAML_VSCODE_SCENARIO;
const record = event => appendFileSync(process.env.DAML_LSP_STATUS_FILE,
  `${JSON.stringify({ session: process.pid, time: Date.now(), ...event })}\n`);
record({ event: 'wrapper-start', pid: process.pid, executable: process.argv[1] });
const child = spawn(process.env.DAML_LSP_PATH, process.argv.slice(2), { stdio: ['pipe', 'pipe', 'inherit'] });
record({ event: 'child-spawn', pid: child.pid });
let initializeId;
let failedShutdownId;
process.stdin.on('data', frames((message, frame) => {
  record({ event: 'outbound', message });
  if (message.method === 'initialize') initializeId = message.id;
  let forwarded = frame;
  if (scenario === 'retirement-failure' && message.method === 'shutdown') {
    // Deliberate peer fault: forward the correlated request to the real native
    // server under an unsupported method. Its genuine error reply, not a fake
    // fixture response, makes the client's shutdown attempt fail.
    failedShutdownId = message.id;
    const faultyRequest = { ...message, method: 'daml.verify/unsupported-shutdown' };
    const body = Buffer.from(JSON.stringify(faultyRequest));
    forwarded = Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
    record({ event: 'shutdown-method-fault', message: faultyRequest });
  }
  if (!child.stdin.destroyed) child.stdin.write(forwarded);
}));
process.stdin.on('end', () => child.stdin.end());
child.stdout.on('data', frames((message, frame) => {
  if (failedShutdownId !== undefined && message.id === failedShutdownId && message.method === undefined) {
    record({ event: 'shutdown-fault-response', message });
  }
  if (process.env.DAML_LSP_WITHHOLD_INITIALIZE === '1' && message.id === initializeId && message.method === undefined) {
    record({ event: 'initialize-response-withheld' });
    return;
  }
  process.stdout.write(frame);
}));
child.stdin.on('error', error => {
  if (error.code !== 'EPIPE' && error.code !== 'ERR_STREAM_DESTROYED') throw error;
});
child.once('error', error => { record({ event: 'spawn-error', message: error.message }); process.exit(1); });
child.once('exit', (code, signal) => { record({ event: 'child-exit', code, signal }); process.exit(code ?? 1); });
if (scenario === 'crash-lineage') {
  // A control file targets one initialized real process, never a timer racing
  // bootstrap. Every replacement uses the same configured executable path.
  const control = process.env.DAML_LSP_CRASH_CONTROL;
  watch(control, () => {
    const target = Number(readFileSync(control, 'utf8'));
    if (target === process.pid) {
      record({ event: 'crash-requested', child: child.pid });
      child.kill('SIGKILL');
    }
  });
}
