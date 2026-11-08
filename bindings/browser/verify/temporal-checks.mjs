export function runTemporalChecks(sdk) {
  const document = sdk.parse(`{\n  date: !Date "2024-02-29"\n  time: !Time "09:30:00.123456789123"\n  local: !DateTime "2026-10-29T09"\n  utc: !DateTime "2026-10-29T09Z"\n  plain: "2024-02-29"\n}`);
  const value = document.value();
  const field = (node, key) => node.entries.find((entry) => entry.key === key)?.value;
  const date = field(value, 'date');
  const time = field(value, 'time');
  const local = field(value, 'local');
  const utc = field(value, 'utc');
  const plain = field(value, 'plain');
  const same = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(`${label} mismatch: ${JSON.stringify(actual)}`);
    }
  };
  same(date, { kind: 'date', value: '2024-02-29' }, 'date semantic wire value');
  same(time, { kind: 'time', value: '09:30:00.123456789123' }, 'time semantic wire value');
  same(local, { kind: 'datetime', value: '2026-10-29T09:00:00' }, 'local datetime semantic wire value');
  same(utc, { kind: 'datetime', value: '2026-10-29T09:00:00Z' }, 'UTC datetime semantic wire value');
  same(plain, { kind: 'string', value: '2024-02-29' }, 'ordinary string semantic wire value');
  const roundtrip = sdk.parse(sdk.write(value));
  const roundtripValue = roundtrip.value();
  same(field(roundtripValue, 'date'), { kind: 'date', value: '2024-02-29' }, 'round-trip date');
  same(field(roundtripValue, 'time'), { kind: 'time', value: '09:30:00.123456789123' }, 'round-trip precise time');
  same(field(roundtripValue, 'local'), { kind: 'datetime', value: '2026-10-29T09:00:00' }, 'round-trip local datetime');
  same(field(roundtripValue, 'utc'), { kind: 'datetime', value: '2026-10-29T09:00:00Z' }, 'round-trip UTC datetime');
  same(field(roundtripValue, 'plain'), { kind: 'string', value: '2024-02-29' }, 'round-trip ordinary string');
  const formatted = document.format();
  if (!formatted.includes('09:30:00.123456789123')) throw new Error('source formatter lost authored fraction');
  document.free();
  roundtrip.free();
  return { temporalKinds: 3, temporalPrecision: '123456789123', temporalWireRoundTrip: true, ...runCronChecks(sdk) };
}

function runCronChecks(sdk) {
  const same = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(`cron ${label}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
    }
  };
  const field = (value, key) => value.entries.find((entry) => entry.key === key).value;
  const rejects = (operation, category) => {
    let caught;
    try { operation(); } catch (error) { caught = error; }
    if (!caught || caught.category !== category) {
      throw new Error(`cron expected ${category} rejection, got ${caught?.category}`);
    }
  };
  const expected = { kind: 'cron', value: '0 0 0 * * 0' };
  const document = sdk.parse('!Cron " @weekly "');
  same(document.value(), expected, 'parsed wire');
  if (!document.format().includes('" @weekly "')) throw new Error('cron lost authored source');
  const written = sdk.write(expected);
  const roundtrip = sdk.parse(written);
  same(roundtrip.value(), expected, 'semantic write');
  rejects(() => sdk.parse('!Cron "not a cron expression"'), 'parse');
  rejects(() => sdk.write({ kind: 'cron', value: '0 0 24 * * *' }), 'semantic');
  const bundle = new sdk.Bundle('.', {
    'daml.bundle': '{}',
    'Schedule.daml': `!Object {
when: !Cron
text: !String
} = {
when: !Cron "@weekly"
text: "ordinary"
}`,
  });
  const input = sdk.parse('!self.Schedule');
  const schemas = bundle.compile(input);
  const result = schemas.resolve(input);
  same(field(result.value(), 'when'), expected, 'resolved default');
  const invalid = sdk.parse('!self.Schedule { text: !Cron "@daily" }');
  rejects(() => schemas.resolve(invalid), 'resolve');
  let sawSummary = false;
  const stage = {
    defer(summary, path) {
      if (path === '/when') {
        same(summary, expected, 'staged shallow summary');
        sawSummary = true;
      }
      return false;
    },
    materialize(value, copies, round) {
      same(field(value, 'when'), expected, 'staged snapshot');
      return round === 0 ? [{ op: 'copy', path: '/copied', from: '/when' }] : null;
    },
  };
  const staged = schemas.resolveStaged(input, stage);
  if (!sawSummary) throw new Error('cron shallow summary not exposed');
  same(field(staged.value(), 'copied'), expected, 'stage copy');
  const semantic = schemas.resolveSemanticStaged(result.value(), {
    defer() { return false; },
    materialize() { return null; },
  });
  same(field(semantic.value(), 'when'), expected, 'semantic re-admission');
  const snapshot = result.value();
  input.free();
  schemas.free();
  bundle.free();
  same(field(result.value(), 'when'), expected, 'owned snapshot');
  same(field(snapshot, 'when'), expected, 'exported snapshot');
  document.free();
  roundtrip.free();
  invalid.free();
  result.free();
  staged.free();
  semantic.free();
  return { cronWireRoundTrip: true, cronTypedDefault: true, cronStagedSummary: true };
}
