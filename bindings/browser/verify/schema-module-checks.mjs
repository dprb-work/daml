// Consumer-generated JSON Schema modules must share native semantics, not a browser validator.
export function runSchemaModuleChecks(sdk, fixture, nativeResult) {
  const ready = { defer() { return false; }, materialize() { return null; } };
  const fail = (message) => { throw new Error(`schema module contract: ${message}`); };
  const schemaError = (action) => {
    try {
      const unexpected = action();
      unexpected?.free?.();
    } catch (error) {
      if (error.name !== 'DamlError' || error.category !== 'schema') {
        fail(`expected a schema DamlError, got ${error?.name}/${error?.category}`);
      }
      return;
    }
    fail('invalid schema modules were admitted');
  };
  const originalException = (input, sentinel) => {
    let actual;
    try {
      const unexpected = sdk.Schemas.fromJsonModules(input);
      unexpected.free();
    } catch (error) {
      actual = error;
    }
    if (actual !== sentinel) fail('array access exception identity changed');
  };

  const sources = [...fixture.schemaModuleSources];
  const schemas = sdk.Schemas.fromJsonModules(sources);
  // Retained schemas must not borrow the caller's mutable array or JSON strings.
  sources[0] = '{}';
  sources.length = 0;
  const document = sdk.parse(fixture.schemaModuleSource);
  const resolution = schemas.resolve(document, fixture.schemaModuleExpected);
  try {
    const value = resolution.value();
    if (value.kind !== 'map') fail('generated object did not resolve to a map');
    const fields = new Map(value.entries.map(({ key, value }) => [key, value]));
    if (fields.get('label')?.value !== 'worker' ||
        fields.get('attempts')?.value !== '2' ||
        fields.get('limit')?.value !== '9007199254740993') {
      fail('registered resource defaults or exact large integer were lost');
    }
    if (schemas.write(value, 80) !== nativeResult.canonicalSource) {
      fail('browser completion/canonical output differs from native JSON-module core');
    }
    const origin = resolution.origins()['/attempts'];
    if (origin?.kind !== 'default' ||
        origin.resource !== nativeResult.defaultOrigin.resource ||
        origin.pointer !== nativeResult.defaultOrigin.pointer) {
      fail('cross-module resource default provenance differs from native');
    }
    const invalid = sdk.parse('!app.GeneratedJob {\n label: "worker"\n attempts: -1\n}');
    try {
      let rejected = false;
      try {
        const unexpected = schemas.resolve(invalid, fixture.schemaModuleExpected);
        unexpected.free();
      } catch (error) {
        if (error.name !== 'DamlError' || error.category !== 'resolve' || error.pointer !== '/attempts') {
          fail('registered resource constraint rejection lost its input path');
        }
        rejected = true;
      }
      if (!rejected) fail('negative attempts bypassed the registered resource minimum');
    } finally {
      invalid.free();
    }
    const emptyDocument = sdk.parse('!app.RequiredContainers {}');
    const original = emptyDocument.value();
    const originalSnapshot = JSON.stringify(original);
    const expectValidationError = (input, expected, pointer) => {
      let rejected = false;
      try {
        schemas.validateSemantic(input, expected);
      } catch (error) {
        if (error.name !== 'DamlError' || error.category !== 'resolve' || error.pointer !== pointer) {
          fail('constraint-only validation lost its category or instance pointer');
        }
        rejected = true;
      }
      if (!rejected) fail('constraint-only validation completed or ignored invalid data');
    };
    try {
      expectValidationError(original, 'app.RequiredContainers', '');
      const completed = schemas.resolveSemanticStaged(original, ready, 'app.RequiredContainers');
      try {
        const completedValue = completed.value();
        const completedFields = completedValue.entries.map(({ key }) => key).sort().join(',');
        if (completedFields !== 'configs,items,mode') fail('resolution should still complete containers/defaults');
        schemas.validateSemantic(completedValue, 'app.RequiredContainers');
        expectValidationError(completedValue, 'missing.Type', '');
      } finally {
        completed.free();
      }
      if (JSON.stringify(original) !== originalSnapshot) fail('validation mutated its caller input');
      const nestedDocument = sdk.parse(
        '!app.RequiredContainers {\n configs: !app.GeneratedJob {\n label: "nested"\n attempts: -1\n limit: 5\n}\n items: []\n mode: "safe"\n}',
      );
      try {
        expectValidationError(nestedDocument.value(), 'app.RequiredContainers', '/configs/attempts');
      } finally {
        nestedDocument.free();
      }
      const literal = {
        kind: 'map',
        entries: [{ key: '$ref', value: { kind: 'string', value: 'literal-data-not-a-schema-reference' } }],
      };
      schemas.validateSemantic(literal, 'app.LiteralKeys');
      expectValidationError({
        ...literal, entries: [{ key: '$ref', value: { kind: 'number', value: '12' } }],
      }, 'app.LiteralKeys', '/$ref');
      // Validation does not establish source representability; resolution still enforces it.
      let sourceRejected = false;
      try {
        const unexpected = schemas.resolveSemanticStaged(literal, ready, 'app.LiteralKeys');
        unexpected.free();
      } catch (error) {
        if (error.name !== 'DamlError' || error.category !== 'resolve') {
          fail('literal-key source representability failure changed category');
        }
        sourceRejected = true;
      }
      if (!sourceRejected) fail('validation-only admission weakened ordinary semantic source rules');
    } finally {
      emptyDocument.free();
    }
  } finally {
    document.free();
    schemas.free();
  }
  // Resolutions remain owned after the source and registry handles are released.
  if (resolution.value().entries.find(({ key }) => key === 'attempts')?.value.value !== '2') {
    fail('resolution became invalid after freeing its schema registry');
  }
  resolution.free();

  const treeSchemas = sdk.Schemas.fromJsonModules([fixture.recursiveModule]);
  const treeDocument = sdk.parse(fixture.recursiveSource);
  let treeResolution;
  try {
    const before = treeDocument.value();
    let missingRejected = false;
    try {
      treeSchemas.validateSemantic(before, fixture.recursiveExpected);
    } catch (error) {
      if (error.category !== 'resolve' || error.kind !== 'SchemaViolation') {
        fail('recursive validation did not preserve missing-field rejection');
      }
      missingRejected = true;
    }
    if (!missingRejected) fail('recursive validation applied completion');
    treeResolution = treeSchemas.resolve(treeDocument, fixture.recursiveExpected);
    const completed = treeResolution.value();
    treeSchemas.validateSemantic(completed, fixture.recursiveExpected);
    if (treeSchemas.write(completed) !== nativeResult.recursiveCanonicalSource) {
      fail('recursive completion/source differs between native and browser');
    }
    const next = completed.entries.find(({ key }) => key === 'next').value;
    const copies = next.entries.find(({ key }) => key === 'copies').value;
    if (copies.value !== '9007199254740993') fail('recursive default lost exact numeric data');
    const origin = treeResolution.origins()['/next/copies'];
    if (origin.kind !== 'default' ||
        origin.resource !== nativeResult.recursiveDefaultOrigin.resource ||
        origin.pointer !== nativeResult.recursiveDefaultOrigin.pointer) {
      fail('recursive default provenance differs between native and browser');
    }
    next.entries.find(({ key }) => key === 'value').value.value = '-1';
    let invalidRejected = false;
    try {
      treeSchemas.validateSemantic(completed, fixture.recursiveExpected);
    } catch (error) {
      if (error.category !== 'resolve' || error.kind !== 'SchemaViolation' || error.pointer !== '/next/value') {
        fail('recursive descendant failure lost its category/path');
      }
      invalidRejected = true;
    }
    if (!invalidRejected) fail('recursive descendant constraint was ignored');
    if (JSON.stringify(treeDocument.value()) !== JSON.stringify(before)) {
      fail('recursive validation/resolution mutated the document');
    }
  } finally {
    treeResolution?.free();
    treeDocument.free();
    treeSchemas.free();
  }
  schemaError(() => sdk.Schemas.fromJsonModules([
    '{"types":{"app.Loop":{"schema":{"allOf":[{"$ref":"#"}]}}}}',
  ]));

  const endless = sdk.Schemas.fromJsonModules([
    '{"types":{"app.Endless":{"schema":{"type":"object","properties":{"child":{"$ref":"#"}},"required":["child"]}}}}',
  ]);
  const empty = sdk.parse('{}');
  let endlessResult;
  let bounded = false;
  try {
    try {
      endlessResult = endless.resolve(empty, 'app.Endless');
    } catch (error) {
      if (error.category !== 'resolve' || error.kind !== 'LimitExceeded') {
        fail('unending recursive completion trapped instead of returning its limit error');
      }
      bounded = true;
    }
  } finally {
    endlessResult?.free();
    empty.free();
    endless.free();
  }
  if (!bounded) fail('unending recursive completion invented a successful leaf');

  schemaError(() => sdk.Schemas.fromJsonModules(['{']));
  schemaError(() => sdk.Schemas.fromJsonModules([fixture.schemaModuleSources[1]]));
  schemaError(() => sdk.Schemas.fromJsonModules([
    ...fixture.schemaModuleSources, fixture.schemaModuleSources[1],
  ]));
  schemaError(() => sdk.Schemas.fromJsonModules([42]));
  schemaError(() => sdk.Schemas.fromJsonModules([
    '{"types":{"app.Text":{"schema":{"const":"' + '\ud800' + '"}}}}',
  ]));
  schemaError(() => sdk.Schemas.fromJsonModules(new Array(10_001).fill('{}')));
  // Each empty module is independently valid; only aggregate source bytes exceed the cap.
  const padded = ' '.repeat(4 * 1024 * 1024) + '{}';
  schemaError(() => sdk.Schemas.fromJsonModules([padded, padded]));

  const indexSentinel = { boundary: 'module index getter' };
  const indexThrows = [];
  Object.defineProperty(indexThrows, '0', { get() { throw indexSentinel; } });
  originalException(indexThrows, indexSentinel);
  const lengthSentinel = { boundary: 'module array length getter' };
  const lengthThrows = new Proxy([], {
    get(target, key, receiver) {
      if (key === 'length') throw lengthSentinel;
      return Reflect.get(target, key, receiver);
    },
  });
  originalException(lengthThrows, lengthSentinel);
  return { schemaModuleParity: true, schemaModuleAdmissionBounds: true, structuralRecursionParity: true, recursiveCompletionBound: true };
}
