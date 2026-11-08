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
  return { temporalKinds: 3, temporalPrecision: '123456789123', temporalWireRoundTrip: true };
}
