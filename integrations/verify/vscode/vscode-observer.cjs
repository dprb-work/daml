const Module = require('node:module');
const vscode = require('vscode');

function reflection(target) {
  return {
    ownKeys() { return Reflect.ownKeys(target); },
    getOwnPropertyDescriptor(_receiver, key) {
      const descriptor = Reflect.getOwnPropertyDescriptor(target, key);
      return descriptor && { ...descriptor, configurable: true };
    },
    has(_receiver, key) { return Reflect.has(target, key); },
  };
}

// Observe only selected real consumers. Every resource is created/disposed by
// VS Code; proxies bind all other methods to their original API receivers.
module.exports = function observeVscode(acceptParent) {
  const channels = [];
  const watchers = [];
  const logs = [];
  const reads = [];
  const pendingEvents = [];
  const load = Module._load;
  let batching = false;
  let snapshot;
  function resource(target, record, output = false) {
    function operation(method, log) {
      if (!output) return;
      record.operationCount++;
      record.operations.push({ method, ...(log === undefined ? {} : { log }) });
      if (record.operations.length > 64) record.operations.shift();
    }
    return new Proxy({}, {
      ...reflection(target),
      get(_receiver, key) {
        const object = target;
        if (key === 'dispose') return () => { const result = object.dispose(); record.disposed = true; operation('dispose'); return result; };
        if (output && (key === 'append' || key === 'appendLine')) return text => {
          const result = object[key](text);
          logs.push(String(text));
          operation(key, logs.length - 1);
          return result;
        };
        if (output && ['replace', 'clear', 'show', 'hide'].includes(key)) return (...args) => {
          const result = object[key](...args);
          if (key === 'replace') logs.push(String(args[0]));
          operation(key, key === 'replace' ? logs.length - 1 : undefined);
          return result;
        };
        const value = Reflect.get(object, key, object);
        return typeof value === 'function' ? value.bind(object) : value;
      },
    });
  }
  const window = new Proxy({}, {
    ...reflection(vscode.window),
    get(_receiver, key) {
      const target = vscode.window;
      if (key === 'createOutputChannel') return (...args) => {
        const channel = target.createOutputChannel(...args);
        const record = { channel, disposed: false, name: args[0], operations: [{ method: 'create' }], operationCount: 1 };
        channels.push(record);
        return resource(channel, record, true);
      };
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  const workspace = new Proxy({}, {
    ...reflection(vscode.workspace),
    get(_receiver, key) {
      const target = vscode.workspace;
      if (key === 'createFileSystemWatcher') return (...args) => {
        const watcher = target.createFileSystemWatcher(...args);
        const record = { watcher, disposed: false, pattern: args[0] };
        watchers.push(record);
        return resource(watcher, record);
      };
      if (key === 'getConfiguration') return (...args) => {
        if (snapshot && args[0] === 'definitivelyAssertedDaml') {
          reads.push(snapshot.get('serverPath'));
          return snapshot;
        }
        return target.getConfiguration(...args);
      };
      if (key === 'onDidChangeConfiguration') return (listener, thisArg, disposables) => target.onDidChangeConfiguration(event => {
        if (batching && event.affectsConfiguration('definitivelyAssertedDaml.serverPath')) {
          let entry = pendingEvents.find(candidate => candidate.event === event);
          if (!entry) {
            entry = { event, snapshot: target.getConfiguration('definitivelyAssertedDaml'), listeners: [] };
            pendingEvents.push(entry);
          }
          entry.listeners.push(() => listener.call(thisArg, event));
        } else listener.call(thisArg, event);
      }, undefined, disposables);
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  const observed = new Proxy({}, {
    ...reflection(vscode),
    get(_receiver, key) {
      const target = vscode;
      if (key === 'window') return window;
      if (key === 'workspace') return workspace;
      return Reflect.get(target, key, target);
    },
  });
  Module._load = function (request, parent, isMain) {
    if (request === 'vscode' && acceptParent(parent?.filename)) return observed;
    return Reflect.apply(load, this, [request, parent, isMain]);
  };
  return {
    channels, watchers, logs, reads, pendingEvents,
    beginConfigurationBatch() { batching = true; },
    flushConfigurationBatch() {
      batching = false;
      try {
        // No await: these genuine callbacks run in one JavaScript delivery batch.
        for (const entry of pendingEvents) {
          snapshot = entry.snapshot;
          for (const deliver of entry.listeners) deliver();
        }
      } finally { snapshot = undefined; }
    },
    restore() { Module._load = load; },
  };
};
