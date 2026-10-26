// Ordinary byte views, throwing array accessors, and public-handle cleanup.
export function runReviewChecks(sdk) {
  const fail = (message) => { throw new Error(`review browser contract: ${message}`); };
  const ready = { defer() { return false; }, materialize() { return null; } };

  const withContext = (check) => {
    const bundle = new sdk.Bundle('.', { 'daml.bundle': '{}' });
    const document = sdk.parse('null');
    const schemas = bundle.compile(document);
    let failed = false;
    let failure;
    try {
      check({ bundle, document, schemas });
    } catch (error) {
      failed = true;
      failure = error;
    }
    let cleanupFailure;
    for (const [name, handle] of [
      ['schemas', schemas], ['document', document], ['bundle', bundle],
    ]) {
      try {
        handle.free();
      } catch (error) {
        cleanupFailure ??= new Error(`public ${name} handle must remain normally freeable`, { cause: error });
      }
    }
    if (failed) throw failure;
    if (cleanupFailure) throw cleanupFailure;
  };

  // A non-array proxy is ordinary callback output. Prototype inspection must
  // preserve its original exception rather than silently classifying it as an
  // invalid stage result.
  withContext(({ schemas, document }) => {
    const sentinel = { case: 'materializer proxy prototype' };
    const result = new Proxy({}, {
      getPrototypeOf() { throw sentinel; },
    });
    let caught;
    try {
      schemas.resolveStaged(document, {
        ...ready, materialize() { return result; },
      });
    } catch (error) {
      caught = error;
    }
    if (caught !== sentinel) fail('materializer proxy exception identity was not retained');
  });

  // The own getter always throws. No invented or changing sizes are involved;
  // on the old implementation this exits before any typed-array copy is reached.
  withContext(({ schemas, document }) => {
    const sentinel = new Error('own byte-view length accessor sentinel');
    const bytes = new Uint8Array([7, 0, 255]);
    Object.defineProperty(bytes, 'length', { get() { throw sentinel; } });
    const resolved = schemas.resolveStaged(document, {
      ...ready, materialize() { return bytes; },
    });
    try {
      const actual = Array.from(resolved.meaning());
      if (actual.length !== 3 || actual[0] !== 7 || actual[1] !== 0 || actual[2] !== 255) {
        fail('Ready meaning must retain the actual bytes of a genuine byte view');
      }
    } finally {
      resolved.free();
    }
  });

  const indexThrows = (sentinel) => {
    const values = [];
    Object.defineProperty(values, '0', {
      enumerable: true,
      get() { throw sentinel; },
    });
    return values;
  };
  const lengthThrows = (sentinel) => new Proxy([], {
    get(target, key, receiver) {
      if (key === 'length') throw sentinel;
      return Reflect.get(target, key, receiver);
    },
  });
  const expectOriginal = (operation, sentinel) => {
    let caught;
    let unexpected;
    try {
      unexpected = operation();
    } catch (error) {
      caught = error;
    }
    if (unexpected) unexpected.free();
    if (caught !== sentinel) fail('an array accessor failure must propagate its original thrown value');
  };

  // Both reads occur after materialize has returned, not inside its call.
  for (const makeArray of [indexThrows, lengthThrows]) {
    withContext(({ schemas, document }) => {
      const sentinel = { case: 'returned edit-array accessor' };
      const edits = makeArray(sentinel);
      expectOriginal(() => schemas.resolveStaged(document, {
        ...ready, materialize() { return edits; },
      }), sentinel);
    });
  }

  withContext(({ schemas }) => {
    const sentinel = { case: 'semantic list item accessor' };
    expectOriginal(() => schemas.resolveSemanticStaged({
      kind: 'list', items: indexThrows(sentinel),
    }, ready), sentinel);
  });

  withContext(({ schemas }) => {
    const sentinel = { case: 'semantic map entries length accessor' };
    expectOriginal(() => schemas.resolveSemanticStaged({
      kind: 'map', entries: lengthThrows(sentinel),
    }, ready), sentinel);
  });

  withContext(({ schemas, document }) => {
    const sentinel = { case: 'initial edit-array length accessor' };
    expectOriginal(() => schemas.resolveStaged(
      document, ready, undefined, lengthThrows(sentinel),
    ), sentinel);
  });

  withContext(({ bundle, document }) => {
    const sentinel = { case: 'additional-type element accessor' };
    expectOriginal(() => bundle.compile(
      document, undefined, indexThrows(sentinel),
    ), sentinel);
  });

  return { review: 'passed' };
}
