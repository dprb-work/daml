import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const here = fileURLToPath(new URL('.', import.meta.url));
const args = new Set(process.argv.slice(2));
const option = (name, fallback) => {
  const match = process.argv.slice(2).find((arg) => arg.startsWith(`${name}=`));
  return match ? match.slice(name.length + 1) : fallback;
};
const executablePath = process.env.CHROMIUM_PATH ?? process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
if (!executablePath) {
  throw new Error('Set CHROMIUM_PATH to an installed Chromium executable; browser downloads are disabled.');
}
const dist = resolve(option('--dist', resolve(here, '../dist')));
const fixture = JSON.parse(await readFile(resolve(here, 'fixtures/native-parity.json'), 'utf8'));
const nativeResultPath = option('--native-result', null);
if (!nativeResultPath) {
  throw new Error('Pass --native-result=<JSON output from the native-parity example> to run real native/browser parity.');
}
const nativeResult = JSON.parse(await readFile(resolve(nativeResultPath), 'utf8'));
if (typeof nativeResult.canonicalSource !== 'string' ||
    typeof nativeResult.taglessCanonicalSource !== 'string' ||
    typeof nativeResult.rebaseCanonicalSource !== 'string' ||
    typeof nativeResult.defaultOrigin?.resource !== 'string' ||
    typeof nativeResult.defaultOrigin?.pointer !== 'string') {
  throw new Error('Native parity result must contain canonical sources and actual default-origin resource/pointer metadata.');
}
const html = `<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,"><title>DAML browser verifier</title><main id="result">Loading SDK</main>`;
const contentTypes = { '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.wasm': 'application/wasm' };
const server = createServer(async (request, response) => {
  try {
    const pathname = new URL(request.url, 'http://127.0.0.1').pathname;
    if (pathname === '/') {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      response.end(html);
      return;
    }
    const verifierModule = pathname === '/staged-checks.mjs' ||
      pathname === '/review-checks.mjs' || pathname === '/final-review-checks.mjs' ||
      pathname === '/temporal-checks.mjs';
    const file = verifierModule
      ? resolve(here, pathname.slice(1))
      : resolve(dist, `.${decodeURIComponent(pathname)}`);
    if (!verifierModule && file !== dist && !file.startsWith(`${dist}${sep}`)) {
      response.writeHead(400).end();
      return;
    }
    const body = await readFile(file);
    response.writeHead(200, {
      'content-type': contentTypes[extname(file)] ?? 'application/octet-stream',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
    });
    response.end(body);
  } catch {
    response.writeHead(404).end('not found');
  }
});

await new Promise((resolveListen, reject) => {
  server.once('error', reject);
  server.listen(0, '127.0.0.1', resolveListen);
});
const address = server.address();
const origin = `http://127.0.0.1:${address.port}`;
const browser = await chromium.launch({
  headless: !args.has('--headed'),
  executablePath,
  ...(args.has('--headed') && option('--cdp-port', null)
    ? {
        args: [`--remote-debugging-port=${option('--cdp-port', '9222')}`],
      }
    : {}),
});
const context = await browser.newContext();
let networkDisabled = false;
const preloadExternalRequests = [];
const postInitRequests = [];
await context.route('**/*', async (route) => {
  const url = new URL(route.request().url());
  if (!networkDisabled && url.origin === origin) {
    await route.continue();
  } else {
    (networkDisabled ? postInitRequests : preloadExternalRequests).push(url.href);
    await route.abort('blockedbyclient');
  }
});
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', (error) => pageErrors.push(error.message));

try {
  await page.goto(origin, { waitUntil: 'domcontentloaded' });
  await page.evaluate(async () => {
    const sdk = await import('/daml_browser.js');
    await sdk.default('/daml_browser_bg.wasm');
    window.sdk = sdk;
    const { runStagedChecks } = await import('/staged-checks.mjs');
    window.runStagedChecks = runStagedChecks;
    const { runReviewChecks } = await import('/review-checks.mjs');
    window.runReviewChecks = runReviewChecks;
    const { runTemporalChecks } = await import('/temporal-checks.mjs');
    window.runTemporalChecks = runTemporalChecks;
    const { runFinalReviewChecks } = await import('/final-review-checks.mjs');
    window.runFinalReviewChecks = runFinalReviewChecks;
    document.querySelector('#result').textContent = 'SDK initialized';
  });
  networkDisabled = true;
  const result = await page.evaluate(({ fixture, nativeCanonical, taglessCanonical, rebaseCanonical, nativeDefaultOrigin }) => {
    const fail = (message) => { throw new Error(`browser contract: ${message}`); };
    const same = (actual, expected, path = '$') => {
      if (actual === expected) return;
      if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') {
        fail(`${path}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
      const actualKeys = Object.keys(actual).sort();
      const expectedKeys = Object.keys(expected).sort();
      if (actualKeys.length !== expectedKeys.length || actualKeys.some((key, i) => key !== expectedKeys[i])) {
        fail(`${path}: shape mismatch ${JSON.stringify(actualKeys)} vs ${JSON.stringify(expectedKeys)}`);
      }
      for (const key of expectedKeys) same(actual[key], expected[key], `${path}.${key}`);
    };
    const semanticView = (node, path = '$') => {
      const result = { kind: node.kind };
      if (node.tag) {
        if (typeof node.tag.name !== 'string' ||
            (node.tag.binding !== undefined && typeof node.tag.binding !== 'string')) {
          fail(`${path}: invalid semantic tag metadata`);
        }
        result.tag = { name: node.tag.name };
      }
      switch (node.kind) {
        case 'null': break;
        case 'bool':
        case 'number':
        case 'string':
          result.value = node.value;
          break;
        case 'list':
          result.items = node.items.map((item, index) => semanticView(item, `${path}/${index}`));
          break;
        case 'map':
          result.entries = node.entries.map((entry) => ({
            key: entry.key,
            value: semanticView(entry.value, `${path}/${entry.key}`),
          }));
          break;
        default:
          fail(`${path}: unknown semantic kind ${node.kind}`);
      }
      return result;
    };
    const errorIs = (action, category) => {
      try { action(); } catch (error) {
        if (error.name !== 'DamlError' || error.category !== category) {
          fail(`expected DamlError/${category}, got ${error.name}/${error.category}`);
        }
        return error;
      }
      fail(`expected ${category} error`);
    };
    const sdk = window.sdk;
    const formattedDefinition = sdk.formatDefinition(fixture.sources['Config.daml']);
    if (!formattedDefinition.endsWith('\n')) fail('definition formatting must end in LF');
    if (sdk.formatDefinition(formattedDefinition) !== formattedDefinition) fail('definition formatting must be stable');

    const document = sdk.parse(fixture.source);
    if (document.source !== fixture.source) fail('Document.source must retain exact source');
    const formatted = document.format(80);
    if (!formatted.endsWith('\n')) fail('document formatting must end in LF');
    same(sdk.parse(formatted).value(), document.value(), 'formatted document value');

    const bundle = new sdk.Bundle(fixture.root, fixture.sources);
    const schemas = bundle.compile(document, fixture.expected);
    const resolution = schemas.resolve(document, fixture.expected);
    same(semanticView(resolution.value()), fixture.expectedValue, 'native fixture semantic value and tags');
    const browserCanonical = schemas.write(resolution.value(), 80);
    if (browserCanonical !== nativeCanonical) {
      fail(`browser semantic export differs from native core fixture\nnative: ${nativeCanonical}\nbrowser: ${browserCanonical}`);
    }
    if (resolution.source !== fixture.source) fail('Resolution.source must retain exact original input');
    const origins = resolution.origins();
    const inputOrigin = origins['/name'];
    if (!inputOrigin || inputOrigin.kind !== 'input' || !inputOrigin.span ||
        !Number.isInteger(inputOrigin.span.start) || !Number.isInteger(inputOrigin.span.end)) {
      fail('input origin must expose its original byte span');
    }
    if (!resolution.source.slice(inputOrigin.span.start, inputOrigin.span.end).includes('"worker"')) {
      fail('input origin span must point into the original source value');
    }
    const origin = origins[fixture.defaultOrigin.pointer];
    if (!origin || origin.kind !== 'default' ||
        origin.resource !== nativeDefaultOrigin.resource) {
      fail('default origin resource must match native core provenance');
    }
    if (origin.pointer !== fixture.defaultOrigin.schemaPointer ||
        origin.pointer !== nativeDefaultOrigin.pointer) {
      fail(`expected native/default schema pointer ${nativeDefaultOrigin.pointer}, got ${origin.pointer}`);
    }
    const taglessDocument = sdk.parse(fixture.taglessSource);
    const taglessResolution = schemas.resolve(taglessDocument, fixture.expected);
    const taglessValue = semanticView(taglessResolution.value());
    const expectedTaglessValue = { ...fixture.expectedValue };
    delete expectedTaglessValue.tag;
    delete taglessValue.tag;
    same(taglessValue, expectedTaglessValue, 'tagless expected-root default completion');
    if (taglessResolution.source !== fixture.taglessSource) {
      fail('tagless resolution must retain original source');
    }
    if (schemas.write(taglessResolution.value(), 80) !== taglessCanonical) {
      fail('tagless browser semantic export differs from native resolve_as probe');
    }
    const taglessDefault = taglessResolution.origins()[fixture.defaultOrigin.pointer];
    if (!taglessDefault || taglessDefault.kind !== 'default') {
      fail('tagless expected-root resolution must retain default provenance');
    }
    const invalidTagless = sdk.parse('{ name: 7 }');
    const invalidResolution = errorIs(
      () => schemas.resolve(invalidTagless, fixture.expected),
      'resolve',
    );
    if (invalidResolution.kind !== 'SchemaViolation') {
      fail(`expected a schema violation for an invalid tagless value, got ${invalidResolution.kind}`);
    }
    taglessDocument.free();
    taglessResolution.free();
    invalidTagless.free();
    const siblingDocument = sdk.parse('!sibling.Config');
    const siblingSchemas = bundle.compile(siblingDocument, 'sibling.Config');
    const siblingResolution = siblingSchemas.resolve(siblingDocument, 'sibling.Config');
    const siblingExpected = {
      kind: 'map',
      tag: { name: 'sibling.Config' },
      entries: [{ key: 'value', value: { kind: 'string', value: 'sibling' } }],
    };
    same(semanticView(siblingResolution.value()), siblingExpected, 'dot-prefixed sibling import');
    const siblingExport = siblingSchemas.write(siblingResolution.value(), 80);
    same(semanticView(sdk.parse(siblingExport).value()), siblingExpected, 'sibling semantic export');
    siblingDocument.free();
    siblingSchemas.free();
    siblingResolution.free();
    const rebaseDocument = sdk.parse(fixture.rebaseSource);
    const rebaseSchemas = bundle.compile(rebaseDocument, fixture.rebaseExpected);
    const rebaseResolution = rebaseSchemas.resolve(rebaseDocument, fixture.rebaseExpected);
    const beforeRebase = rebaseResolution.value();
    const beforeSeed = beforeRebase.entries.find((entry) => entry.key === 'seed')?.value;
    const beforeMarker = beforeRebase.entries.find((entry) => entry.key === 'marker')?.value;
    if (!beforeSeed || beforeSeed.kind !== 'string' || beforeSeed.value !== 'seed' ||
        beforeSeed.tag?.name !== 'self.Base' || typeof beforeSeed.tag.binding !== 'string') {
      fail('resolved default must retain its original private tag binding and seed value');
    }
    if (!beforeMarker || beforeMarker.kind !== 'string' || beforeMarker.value !== 'root' ||
        beforeMarker.tag?.name !== 'app.Base') {
      fail('explicit source field must retain the root-visible app.Base assertion');
    }
    const rebaseExport = rebaseSchemas.write(beforeRebase, 80);
    if (rebaseExport !== rebaseCanonical) {
      fail(`qualified browser export differs from native core output\nnative: ${rebaseCanonical}\nbrowser: ${rebaseExport}`);
    }
    const reAdmittedDocument = sdk.parse(rebaseExport);
    const reAdmittedSchemas = bundle.compile(reAdmittedDocument, fixture.rebaseExpected);
    const reAdmittedResolution = reAdmittedSchemas.resolve(reAdmittedDocument, fixture.rebaseExpected);
    const reAdmittedValue = reAdmittedResolution.value();
    same(semanticView(reAdmittedValue), fixture.rebaseExpectedValue, 'qualified tag rebase output');
    const reAdmittedSeed = reAdmittedValue.entries.find((entry) => entry.key === 'seed')?.value;
    const reAdmittedMarker = reAdmittedValue.entries.find((entry) => entry.key === 'marker')?.value;
    if (!reAdmittedSeed || reAdmittedSeed.kind !== 'string' ||
        reAdmittedSeed.value !== 'seed' || reAdmittedSeed.tag?.name !== 'app.Base' ||
        !reAdmittedMarker || reAdmittedMarker.kind !== 'string' ||
        reAdmittedMarker.value !== 'root' || reAdmittedMarker.tag?.name !== 'app.Base') {
      fail('re-admission must preserve both values through the visible app.Base alias rather than same-shaped root self.Base');
    }
    rebaseDocument.free();
    rebaseSchemas.free();
    rebaseResolution.free();
    reAdmittedDocument.free();
    reAdmittedSchemas.free();
    reAdmittedResolution.free();

    const edited = {
      kind: 'map',
      entries: [
        { key: 'count', value: { kind: 'number', value: '900719925474099312345678901234567890' } },
        { key: 'decimal', value: { kind: 'number', value: '1234567890.01234567890123456789' } },
        { key: 'text', value: { kind: 'string', value: 'line\\break\n雪 🌍' } },
      ],
    };
    const written = sdk.write(edited, 80);
    if (!written.endsWith('\n')) fail('semantic writer must emit trailing LF');
    same(sdk.parse(written).value(), edited, 'exact semantic write/reparse');

    const regex = {
      kind: 'string',
      value: String.raw`\d+\s#literal`,
      tag: { name: 'Regex' },
    };
    same(sdk.parse(sdk.write(regex, 80)).value(), regex, 'raw Regex write/reparse');

    const parseError = errorIs(() => sdk.parse('{ title: "🌍", broken: }'), 'parse');
    if (!parseError.range || !parseError.range.start || !Number.isInteger(parseError.range.start.character)) {
      fail('Unicode parse diagnostic must expose a UTF-16 range');
    }
    errorIs(() => sdk.parse('{ value: 1 }').format(0), 'semantic');
    errorIs(() => sdk.write({ kind: 'map', entries: [
      { key: 'same', value: { kind: 'null' } },
      { key: 'same', value: { kind: 'bool', value: true } },
    ] }, 80), 'semantic');
    const malformedNode = errorIs(
      () => sdk.write({ kind: 'number', value: 'NaN', unknown: true }, 80),
      'semantic',
    );
    if (!malformedNode.kind) fail('invalid lossless node shapes must expose a stable kind');
    const privateBinding = {
      kind: 'map',
      entries: [],
      tag: { name: 'Config', binding: 'urn:private:hidden' },
    };
    const privateError = errorIs(() => schemas.write(privateBinding, 80), 'semantic');
    if (!privateError.kind) fail('unrepresentable private binding must expose a stable kind');

    const scalarTokenError = errorIs(
      () => sdk.write({ kind: 'string', value: '\n'.repeat(600_000) }, 80),
      'semantic',
    );
    if (scalarTokenError.kind !== 'LimitExceeded') {
      fail(`escaped scalar token bound returned ${scalarTokenError.kind}`);
    }
    const oversizedList = Array.from({ length: 100_000 }, () => ({ kind: 'null' }));
    const nodeBudgetError = errorIs(
      () => sdk.write({ kind: 'list', items: oversizedList }, 80),
      'semantic',
    );
    if (nodeBudgetError.kind !== 'LimitExceeded') {
      fail(`semantic node bound returned ${nodeBudgetError.kind}`);
    }
    const oversizedExpectedError = errorIs(
      () => bundle.compile(document, 'x'.repeat(1_048_577)),
      'semantic',
    );
    if (oversizedExpectedError.kind !== 'LimitExceeded') {
      fail(`oversized expected-root admission returned ${oversizedExpectedError.kind}`);
    }
    const unpairedExpectedError = errorIs(
      () => bundle.compile(document, String.fromCharCode(0xd800)),
      'semantic',
    );
    if (!unpairedExpectedError.kind) fail('invalid Unicode expected roots must expose a stable kind');
    const sparseInner = new Array(99_999);
    sparseInner[0] = { kind: 'null' };
    const sparseGraphError = errorIs(
      () => sdk.write({ kind: 'list', items: [{ kind: 'list', items: sparseInner }] }, 80),
      'semantic',
    );
    if (!sparseGraphError.kind) fail('nested sparse semantic graphs must expose a stable kind');

    const resourceEscape = errorIs(() => new sdk.Bundle('.', {
      'daml.bundle': '{}',
      '../escape.daml': '!String',
    }), 'native');
    if (!['PathEscape', 'InvalidManifest'].includes(resourceEscape.kind)) {
      fail(`resource key escape returned ${resourceEscape.kind}`);
    }
    const tooManySources = Object.create(null);
    tooManySources['daml.bundle'] = '{}';
    for (let index = 0; index < 10_000; index += 1) {
      tooManySources[`unused-${index}.daml`] = '';
    }
    const countError = errorIs(() => new sdk.Bundle('.', tooManySources), 'native');
    if (countError.kind !== 'LimitExceeded') fail(`source count bound returned ${countError.kind}`);
    const sizeError = errorIs(() => new sdk.Bundle('.', {
      'daml.bundle': '{}',
      'Unused.daml': 'x'.repeat(8 * 1024 * 1024 + 1),
    }), 'native');
    if (sizeError.kind !== 'LimitExceeded') fail(`source byte bound returned ${sizeError.kind}`);
    const pathError = errorIs(() => new sdk.Bundle('.', {
      'daml.bundle': '{}',
      [`${'a'.repeat(4_092)}.daml`]: '',
    }), 'native');
    if (pathError.kind !== 'LimitExceeded') fail(`source path bound returned ${pathError.kind}`);

    const unresolved = errorIs(() => bundle.compile(sdk.parse('null'), 'self.Missing'), 'native');
    if (!unresolved.kind) fail('native errors must expose a stable kind');
    const rejected = errorIs(() => schemas.resolve(sdk.parse('"wrong type"'), 'self.Config'), 'resolve');
    if (!rejected.kind) fail('resolution errors must expose a stable kind');

    const snapshot = resolution.value();
    const snapshotSource = resolution.source;
    document.free();
    schemas.free();
    same(resolution.value(), snapshot, 'owned resolution snapshot after freeing inputs/context');
    if (resolution.source !== snapshotSource) fail('resolution source must survive freeing inputs/context');
    resolution.free();
    bundle.free();
    return {
      nativeParityCase: fixture.case,
      ...window.runStagedChecks(sdk),
      ...window.runReviewChecks(sdk),
      ...window.runTemporalChecks(sdk),
      ...window.runFinalReviewChecks(sdk),
    };
  }, {
    fixture,
    nativeCanonical: nativeResult.canonicalSource,
    taglessCanonical: nativeResult.taglessCanonicalSource,
    rebaseCanonical: nativeResult.rebaseCanonicalSource,
    nativeDefaultOrigin: nativeResult.defaultOrigin,
  });

  if (preloadExternalRequests.length !== 0) {
    throw new Error(`unexpected external requests during SDK/WASM preload: ${preloadExternalRequests.join(', ')}`);
  }
  if (postInitRequests.length !== 0) {
    throw new Error(`unexpected requests after SDK/WASM preload: ${postInitRequests.join(', ')}`);
  }
  if (pageErrors.length) throw new Error(`browser page errors: ${pageErrors.join('; ')}`);
  console.log(JSON.stringify({ status: 'passed', ...result, postInitRequests: postInitRequests.length, origin }));
  if (args.has('--hold')) {
    console.log(`Persistent browser open at ${origin}; CDP port ${option('--cdp-port', 'none')}`);
    await new Promise((resolveHold) => process.once('SIGINT', resolveHold));
  }
} finally {
  await context.close();
  await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
