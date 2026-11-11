const fs = require('node:fs');
const { createRequire } = require('node:module');
const rpc = createRequire(process.env.DAML_CONSUMER_PACKAGE)('vscode-jsonrpc/node');
const record = event => fs.appendFileSync(process.env.DAML_CONSUMER_EVENTS, `${JSON.stringify(event)}\n`);
const connection = rpc.createMessageConnection(new rpc.StreamMessageReader(process.stdin), new rpc.StreamMessageWriter(process.stdout));
connection.onRequest('initialize', () => ({ capabilities: {} }));
connection.onRequest('shutdown', () => null);
connection.onNotification('exit', () => record({ event: 'protocol-exit' }));
connection.listen();
setInterval(() => {}, 60_000);
fs.watch(process.env.DAML_CONSUMER_CONTROL, () => {
  const command = fs.readFileSync(process.env.DAML_CONSUMER_CONTROL, 'utf8');
  if (command === 'stderr') process.stderr.write('DAML consumer late stderr\n', () => record({ event: 'stderr-sent' }));
  if (command === 'exit') { record({ event: 'native-exit' }); process.exit(0); }
});
