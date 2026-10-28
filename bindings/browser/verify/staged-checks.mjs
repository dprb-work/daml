// Executed in the verifier's real browser after the static SDK preload.
export function runStagedChecks(sdk) {
  const fail = (message) => { throw new Error(`staged browser contract: ${message}`); };
  const same = (actual, expected) => {
    if (actual === expected) return;
    if (actual === null || expected === null ||
        typeof actual !== 'object' || typeof expected !== 'object' ||
        Array.isArray(actual) !== Array.isArray(expected)) {
      fail(`expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    }
    const actualKeys = Object.keys(actual).sort();
    const expectedKeys = Object.keys(expected).sort();
    if (actualKeys.length !== expectedKeys.length ||
        actualKeys.some((key, index) => key !== expectedKeys[index])) {
      fail(`shape mismatch ${JSON.stringify(actualKeys)} vs ${JSON.stringify(expectedKeys)}`);
    }
    for (const key of expectedKeys) same(actual[key], expected[key]);
  };
  const field = (value, key) => value.entries.find((entry) => entry.key === key).value;
  const number = (value) => ({ kind: 'number', value });
  const map = (fields) => ({
    kind: 'map', entries: Object.entries(fields).map(([key, value]) => ({ key, value })),
  });
  const errorIs = (operation, category, kind) => {
    let caught;
    try { operation(); } catch (error) { caught = error; }
    if (caught?.category !== category || caught?.kind !== kind) {
      fail(`expected ${category}/${kind}, got ${caught?.category}/${caught?.kind}`);
    }
    return caught;
  };
  const defer = (summary) => summary.kind === 'map' &&
    summary.fields.some((entry) => entry.key === 'ref');
  const ready = { defer() { return false; }, materialize() { return null; } };
  const bundle = new sdk.Bundle('.', {
    'daml.bundle': '{}',
    'Root.daml': '!Object {\nsource: !Int\ntarget: !Any\n} = {source: 7\ntarget: {context: 1}}',
    'ExpectRoot.daml': '!Object {\nsource: !Int\ntarget: !Any\n} = { source: 7 }',
    'Target.daml': 'closed !Object { count: !Int { min: 1 } } = { count: 4 }',
    'Choice.daml': '(!Object { pick: !String } = { pick: "first" }) | (!Object { pick: !String } = { pick: "second" })',
    'Numbers.daml': '(!Object { n: !Number } = { n: 1 }) | (!Object { n: !Number } = { n: 1.0 })',
  });
  const document = sdk.parse('!self.Root { target: { ref: "/source" } }');
  const schemas = bundle.compile(document, undefined, ['self.Target', 'self.ExpectRoot', 'self.Choice', 'self.Numbers']);
  const owned = [];
  try {
    for (const semanticInput of [false, true]) {
      const history = [];
      const meaning = new Uint8Array([3, 7]);
      const stage = {
        defer(summary, path) {
          if (this !== stage) fail('defer this binding was lost');
          if (path === '' && summary.tag?.name === 'self.Root') {
            const target = summary.fields.find((entry) => entry.key === 'target');
            if (!target) fail('root shallow fields lost the authored target');
            same(target.summary, target.summary.kind === 'map'
              ? { kind: 'map', length: 1 }
              : number('7'));
          }
          return defer(summary);
        },
        materialize(value, copies, round) {
          if (this !== stage) fail('callback this binding was lost');
          same(field(value, 'source'), number('7'));
          history.push({ round, copies });
          if (round === 0) {
            same(field(value, 'target'), map({ ref: { kind: 'string', value: '/source' } }));
            return [{ op: 'copy', path: '/target', from: '/source' }];
          }
          value.entries.length = 0;
          return meaning;
        },
      };
      const result = semanticInput
        ? schemas.resolveSemanticStaged(document.value(), stage)
        : schemas.resolveStaged(document, stage);
      owned.push(result);
      if (result.source !== (semanticInput ? undefined : document.source)) {
        fail('staged source ownership was lost or source was fabricated');
      }
      same(field(result.value(), 'target'), number('7'));
      same(history, [
        { round: 0, copies: [] },
        { round: 1, copies: [{ path: '/target', from: '/source' }] },
      ]);
      same(Array.from(result.meaning()), [3, 7]);
      meaning[0] = 99;
      const first = result.meaning();
      first[1] = 99;
      same(Array.from(result.meaning()), [3, 7]);
      const origins = result.origins();
      same(origins['/target'], origins['/source']);
      if (origins['/target'].kind !== 'default') fail('copy fabricated default provenance');
      if (origins[''].kind !== (semanticInput ? 'semantic' : 'input')) {
        fail('staged root has dishonest source/semantic provenance');
      }
    }

    const targetInput = sdk.parse('!self.ExpectRoot { target: {} }');
    const actual = schemas.resolveStaged(targetInput, ready, undefined, [
      { op: 'expect', path: '/target', name: 'self.Target' },
    ]);
    owned.push(targetInput, actual);
    same(field(actual.value(), 'target'), map({ count: number('4') }));
    same(actual.meaning(), null);
    const invalidTarget = sdk.parse('!self.ExpectRoot { target: { extra: true } }');
    owned.push(invalidTarget);
    errorIs(() => schemas.resolveStaged(invalidTarget, ready, undefined, [
      { op: 'expect', path: '/target', name: 'self.Target' },
    ]), 'resolve', 'SchemaViolation');
    const noTransform = { defer, materialize() { return null; } };
    const count = sdk.parse('{ ref: "/source" }');
    owned.push(count);
    errorIs(() => schemas.resolveStaged(count, noTransform, 'self.Target'), 'resolve', 'SchemaViolation');

    const choices = sdk.parse('!self.Choice');
    owned.push(choices);
    const plans = {
      defer() { return false; },
      materialize(value, copies, round) {
        if (round === 0) {
          this.plan = field(value, 'pick').value;
          return [{ op: 'replace', path: '/pick', value: { kind: 'string', value: 'same' } }];
        }
        return new TextEncoder().encode(this.plan);
      },
    };
    errorIs(() => schemas.resolveStaged(choices, plans), 'resolve', 'AmbiguousDefault');
    const sharedPlans = {
      ...plans,
      materialize(value, copies, round) {
        if (round === 0) return plans.materialize.call(this, value, copies, round);
        return new Uint8Array([1]);
      },
    };
    const shared = schemas.resolveStaged(choices, sharedPlans);
    owned.push(shared);
    same(field(shared.value(), 'pick'), { kind: 'string', value: 'same' });
    same(Array.from(shared.meaning()), [1]);

    const fatal = { sentinel: 'original non-Error thrown value' };
    let caught;
    try {
      schemas.resolveStaged(choices, { ...ready, materialize() { throw fatal; } });
    } catch (error) { caught = error; }
    if (caught !== fatal) fail('fatal callback exception identity was not retained');
    const rejection = new Error('reject first domain plan');
    const accepted = schemas.resolveStaged(choices, {
      ...ready,
      materialize(value) {
        return field(value, 'pick').value === 'first' ? rejection : new Uint8Array([2]);
      },
    });
    owned.push(accepted);
    same(field(accepted.value(), 'pick'), { kind: 'string', value: 'second' });
    same(Array.from(accepted.meaning()), [2]);

    const numbers = sdk.parse('!self.Numbers');
    owned.push(numbers);
    const classified = {
      ...ready,
      numberClass(lexeme) {
        if (this !== classified) fail('numberClass this binding was lost');
        if (typeof lexeme !== 'string') fail('numberClass received a lossy JavaScript number');
        return /[.eE]/.test(lexeme) ? 1 : 0;
      },
    };
    errorIs(() => schemas.resolveStaged(numbers, classified), 'resolve', 'AmbiguousDefault');

    const scalar = sdk.parse('1');
    owned.push(scalar);
    errorIs(() => schemas.resolveStaged(scalar, {
      ...ready, defer() { return 1; },
    }), 'semantic', 'InvalidInput');
    for (const invalid of [true, 2 ** 32]) {
      const error = errorIs(() => schemas.resolveStaged(scalar, {
        ...ready, numberClass() { return invalid; },
      }), 'semantic', 'InvalidInput');
      if (error.pointer !== '') fail('invalid scalar classifier lost its data pointer');
    }
    errorIs(() => schemas.resolveStaged(scalar, {
      ...ready, materialize() { return Promise.resolve(null); },
    }), 'semantic', 'InvalidInput');
    errorIs(() => schemas.resolveStaged(scalar, {
      ...ready, materialize() { return []; },
    }), 'resolve', 'InvalidStage');
    errorIs(() => schemas.resolveStaged(scalar, {
      ...ready, materialize() { return [{ op: 'replace', path: '/absent/n', value: number('2') }]; },
    }), 'resolve', 'InvalidStage');

    const replaced = schemas.resolveStaged(scalar, ready, undefined, [
      { op: 'replace', path: '', value: number('2') },
    ]);
    owned.push(replaced);
    same(replaced.value(), number('2'));
    if (replaced.origins()[''].kind !== 'semantic') fail('replacement fabricated a source span');
    document.free();
    schemas.free();
    bundle.free();
    same(field(shared.value(), 'pick'), { kind: 'string', value: 'same' });
    return { staged: 'passed' };
  } finally {
    for (const handle of owned.reverse()) handle.free();
  }
}
