// Preserve the source-only API's independent value and provenance caps.
export function runFinalReviewChecks(sdk) {
  const fail = (message) => { throw new Error(`final review browser contract: ${message}`); };
  const outerKey = 'k'.repeat(65_000);
  const leaf = 'x'.repeat(1_000_000);
  let value = { kind: 'string', value: leaf };
  for (let index = 0; index < 118; index += 1) {
    value = { kind: 'map', entries: [{ key: 'a', value }] };
  }
  value = { kind: 'map', entries: [{ key: outerKey, value }] };
  const source = sdk.write(value, 80);
  const document = sdk.parse(source);
  const bundle = new sdk.Bundle('.', { 'daml.bundle': '{}' });
  const seed = sdk.parse('null');
  const schemas = bundle.compile(seed);
  let ordinary;
  try {
    // 120 nodes, 119 mapping levels: value/key bytes and provenance-pointer
    // bytes each fit eight MiB, but their sum does not. Do not log the payload.
    try {
      ordinary = schemas.resolve(document);
    } catch (error) {
      fail(`ordinary source resolve must preserve separate caps (${error?.category}/${error?.kind})`);
    }
    let actual = ordinary.value();
    for (const key of [outerKey, ...Array(118).fill('a')]) {
      if (actual.kind !== 'map' || actual.entries.length !== 1 || actual.entries[0].key !== key) {
        fail('ordinary source resolution must preserve the nested value path');
      }
      actual = actual.entries[0].value;
    }
    if (actual.kind !== 'string' || actual.value !== leaf) {
      fail('ordinary source resolution must preserve the complete string leaf');
    }
    const pointer = `/${outerKey}${'/a'.repeat(118)}`;
    const origin = ordinary.origins()[pointer];
    if (origin?.kind !== 'input' || !origin.span ||
        !Number.isInteger(origin.span.start) || !Number.isInteger(origin.span.end)) {
      fail('ordinary source resolution must preserve the leaf input origin');
    }
    const original = ordinary.source;
    if (original !== source || origin.span.end - origin.span.start !== leaf.length + 2 ||
        original[origin.span.start] !== '"' || original[origin.span.end - 1] !== '"' ||
        original.slice(origin.span.start + 1, origin.span.end - 1) !== leaf) {
      fail('leaf origin must identify its actual original quoted source token');
    }

    let staged;
    let boundedFailure;
    try {
      staged = schemas.resolveStaged(document, {
        defer() { return false; },
        materialize() { return null; },
      });
    } catch (error) {
      boundedFailure = error;
    }
    if (staged) staged.free();
    if (boundedFailure?.kind !== 'LimitExceeded' ||
        !['semantic', 'resolve'].includes(boundedFailure.category)) {
      fail('staged resolution must retain its shared cumulative invocation budget');
    }
  } finally {
    ordinary?.free();
    schemas.free();
    document.free();
    seed.free();
    bundle.free();
  }
  return { finalReview: 'passed' };
}
