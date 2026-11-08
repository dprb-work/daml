# Temporal representations: evidence for an input grammar

Reviewed 2026-10-29. Research for API discussion, not an accepted grammar or an
implementation claim. The current [syntax](../syntax.md),
[native definitions](../native-authoring.md) and [Python API](../python.md)
references still describe the implemented language.

## Decision context

The design discussion has agreed on first-class temporal values, using the
existing tagged-payload style: `!Date "..."`, `!Time "..."` and
`!DateTime "..."`. One Time and one DateTime type may carry an explicit offset;
schema constraints can require or forbid it. An absent offset must not acquire
the machine's timezone. Separate LocalTime/LocalDateTime types were not selected.

At the time of this comparison, input spellings, precision, calendar/year bounds,
normalization and Rust/Python exports were still open. The subsequent decision
selected ISO 8601 as the basis; the implemented profile is now owned by the
[syntax reference](../syntax.md#temporal-literals), not by the alternatives below.

## Three different questions

1. **Semantic value:** a calendar date, clock time, local datetime, or datetime
   with an offset. A date alone is not an instant; a clock time still lacks a
   date even if it carries an offset. A named timezone carries rules that a
   numeric offset does not. [RFC 9557 definitions][rfc9557-definitions]
2. **Accepted spelling:** extended/basic notation, omitted seconds, fractional
   digits, separator case, or a different date coordinate system.
3. **Output spelling:** a semantic writer can choose one representation without
   requiring every accepted input to use it. Python and Temporal explicitly
   distinguish accepted input from serialization. [Python parsing][python-parse],
   [Python output][python-output], [Temporal strings][temporal-strings]

These should be decided separately. Several forms below are different semantic
kinds, not alternate spellings of an ordinary complete Date or DateTime.

## Representation families

The final column is a recommendation for discussion, not implemented behavior.

| Family | Example payload or representation | Meaning and evidence | Proposed starting point |
|---|---|---|---|
| Extended calendar date | `2026-10-29` | Complete Gregorian date. RFC 3339 defines this full-date component; JSON Schema uses it for its string date format. [RFC][rfc3339-grammar], [JSON Schema][json-formats] | Default Date spelling. |
| Extended time | `09:30:00`, `09:30:00.125` | Hours/minutes/seconds, optional fractional seconds. No date or implicit timezone. [RFC grammar][rfc3339-grammar], [TOML][toml11] | Default Time spelling, with the already agreed optional explicit offset. |
| Local datetime | `2026-10-29T09:30:00` | Calendar date plus local clock time; not an instant without more information. TOML and EDTF explicitly support it. [TOML][toml11], [EDTF][edtf] | Default offset-free DateTime spelling. Do not call it an RFC 3339 timestamp. |
| Offset-bearing datetime | `2026-10-29T09:30:00Z`, `2026-10-29T09:30:00+01:00` | Date, complete time and UTC relationship. [RFC 3339][rfc3339-grammar] | Default offset-bearing DateTime spellings. |
| Omitted seconds | `09:30`, `2026-10-29T09:30` | TOML 1.1 permits omission and assumes zero seconds; TOML 1.0 and strict RFC 3339 do not. The older W3C profile permits minute-precision datetimes with an offset. [TOML 1.0][toml10], [TOML 1.1][toml11], [W3C][w3c] | Useful authoring convenience; decide explicitly whether it means zero seconds or retained minute precision. |
| Space instead of T | `2026-10-29 09:30:00` | TOML accepts it. RFC 3339 discusses it as an adopting application's readability choice, rather than its defined ABNF. [TOML][toml11], [RFC][rfc3339-grammar] | Reasonable optional input; keep T as the proposed output default. |
| Lowercase separators | `2026-10-29t09:30:00z` | RFC 3339 permits t/z and recommends generators use uppercase. [RFC][rfc3339-grammar] | Reasonable optional input; uppercase output. |
| Basic/compact notation | `20261029`, `20261029T093000` | Historical ISO grammar and Python accept compact forms; RFC 3339 does not, and EDTF expressly requires extended notation. [RFC appendix][rfc3339-appendix], [Python][python-parse], [EDTF][edtf] | Little authoring benefit; leave out unless a concrete input need appears. |
| Week and ordinal dates | `2026-W44-4`, `2026-302` | Different coordinate systems for a complete date. Historical ISO grammar includes both. The Python probe maps the week example to 2026-10-29 but rejects the ordinal form. [RFC appendix][rfc3339-appendix], [Python][python-date] | Valid alternatives worth knowing about; not necessary defaults. Full modern ISO rules were not verified from the paid standard. |
| Reduced-precision dates | `2026`, `2026-10` | Year/month precision, not a complete day. W3C requires adopters to define reduced-precision semantics; EDTF explicitly distinguishes these granularities. [W3C][w3c], [EDTF][edtf] | Do not silently supply January 1 or the first day of the month. Separate semantics would be needed. |
| Other offset spellings | `+0100`, `+01` | Older ISO grammar permits them; strict RFC 3339 uses colon-separated hours and minutes. Python and Pydantic differ on acceptance. [RFC appendix][rfc3339-appendix], [RFC profile][rfc3339-grammar], local probes below | Prefer +01:00. Treat compact offsets as a separate optional-input decision. |
| Decimal comma | `09:30:00,125` | Historical ISO grammar and the observed Python parsers allow it; RFC 3339 uses a dot. [RFC appendix][rfc3339-appendix], [RFC profile][rfc3339-grammar], [Python][python-time] | Prefer a dot; no need to inherit every permissive parser option. |
| Named-zone annotations | `2026-10-29T09:30:00+01:00[Europe/Paris]` | RFC 9557 adds zone annotations to fixed timestamps. A zone is a changing ruleset, not another spelling for +01:00. [RFC 9557][rfc9557-zone], [Temporal][temporal-strings] | Separate capability and semantics; do not silently strip the suffix or equate it with the offset. |
| Component records | JavaScript example: `{ year: 2026, month: 10, day: 29 }` | Temporal accepts property bags as well as strings. This example is not proposed DAML record syntax. [Temporal PlainDate][temporal-date] | An unambiguous alternative constructor shape, but more verbose and a separate payload/API decision. |
| Numeric epoch | A count of seconds, milliseconds or nanoseconds from a declared epoch | ECMAScript Date uses milliseconds; Temporal Instant uses nanoseconds. A count identifies an instant under an explicit convention, not a floating local time or its original offset/zone. [ECMAScript][ecma-time], [Temporal][temporal-strings] | Explicit conversion with declared epoch/unit; never guess the unit from the number's size. |
| Locale-dependent or relative text | `03/04/2026`, `next Friday` | Such text is outside the cited restricted grammars; slash ordering needs a locale, and relative text needs a reference date/context. This consequence is an inference, not a standards classification of every natural-language parser. | Not a language default. A separately selected conversion policy would be needed. |

## Boundaries hidden by familiar spellings

### An offset is not a timezone

`+01:00` records an offset, not a regional rule such as Europe/Paris. Regional
rules can change with daylight saving and legislation. RFC 9557 defines bracketed
zone annotations and discusses contradictions between an offset and a named
zone. Its scope is augmenting a fixed timestamp; it does not define future
floating schedules whose instant should move when timezone rules change.
[RFC 9557 scope][rfc9557-scope], [definitions][rfc9557-definitions],
[inconsistencies][rfc9557-zone]

Therefore adding `[Europe/Paris]` is more than accepting another separator.
We would have to decide whether to retain metadata only or resolve/validate zone
rules, which database/version applies, and how conflicts are handled. Cron and
scheduling policy do not follow automatically from temporal literal support.

### Zero-offset spellings carry a standards nuance

All of `Z`, `+00:00` and RFC's legacy `-00:00` locate the same UTC instant when
attached to a complete datetime, but that does not establish identical metadata.
RFC 9557 updates RFC 3339: `Z` can convey that UTC time is known while the local
offset is unknown; `+00:00` retains the preferred-UTC-reference implication.
It recommends Z in place of the less interoperable -00:00 without formally
deprecating the latter syntax. This is still different from an offset-free local
datetime. [RFC 9557 section 2][rfc9557-zero]

Both observed Python parsers serialize -00:00 as +00:00. Thus accepting a spelling
and preserving its distinction are separate decisions. We should not inherit
normalization behavior accidentally from a host datetime object.

### Fraction precision is a value-model decision

RFC 3339 permits one or more fractional-second digits without imposing a fixed
maximum. Python's datetime/time representation has microsecond precision;
Temporal exposes nanosecond components. TOML requires at least millisecond
support and directs implementations to truncate unsupported extra precision,
not round it. [RFC grammar][rfc3339-grammar], [Python time][python-time],
[Temporal PlainTime][temporal-time], [TOML][toml11]

Our probes observed both Python 3.12 and Pydantic truncate
`09:30:00.123456789` to `09:30:00.123456`. A first-class DAML value therefore
needs a deliberate precision/export policy: reject unrepresentable precision,
preserve it separately, or allow an explicitly requested lossy conversion.
These are alternatives for discussion, not a selected storage representation.

### Midnight, leap seconds and years need explicit limits

RFC 3339 excludes hour 24 but permits actual inserted leap seconds under its
calendar/UTC restrictions. Its scope includes four-digit years 0000 through
9999. The Python probes reject year 0000, 24:00:00 and 23:59:60Z; Python's
datetime model is not the full RFC model. The historical ISO grammar reproduced
in RFC 3339 is broader, but it cannot establish the exact current ISO amendment
rules for midnight and expanded years. [RFC scope][rfc3339-scope],
[restrictions][rfc3339-restrictions], [appendix][rfc3339-appendix],
[Python date][python-date]

Accepting 24:00 may require carrying into the next date; accepting a leap second
requires more than treating 60 as an ordinary second. The API should say what it
supports rather than silently repairing either input.

## Ecosystem comparison

- **RFC 3339:** a restricted timestamp profile, with complete date, complete time,
  an offset, dot fractions and specific leap-second rules. It is not a blanket
  name for every ISO-looking local date/time. [RFC grammar][rfc3339-grammar]
- **TOML:** typed temporal literals, not quoted JSON-style strings. Version 1.0
  requires seconds; 1.1 permits omission. Local and offset datetimes remain
  distinct semantic categories. [1.0][toml10], [1.1][toml11]
- **Python:** fromisoformat accepts spellings that isoformat does not normally
  emit, including compact/week forms and unusually broad separators. This is
  not automatically a suitable language grammar. [Parsing][python-parse],
  [output][python-output]
- **Temporal:** separately models plain date/time/datetime, exact instants and
  zoned datetimes, and accepts strings or component bags. Default property-bag
  overflow can constrain invalid components; it also offers rejection. DAML
  must not inherit clamping merely because an API offers it. [Strings][temporal-strings],
  [PlainDate.from][temporal-date], [PlainTime.from][temporal-time]
- **JSON Schema 2020-12:** date/time/date-time are formats on string instances,
  not temporal JSON kinds. Standard time/date-time reference RFC 3339 full-time
  and date-time and therefore require an offset. Our agreed offset-free values
  cannot simply be exported as if those formats meant local time. Format
  assertion is also a separate vocabulary/validator choice.
  [Formats][json-formats], [vocabularies][json-format-vocab]

## Local observations, not standards claims

One-off calls were run through `bindings/python/.venv/bin/python` using
Python **3.12.3**, Pydantic **2.14.2**, and the standard-library tomllib.
The Python comparison called date/time/datetime.fromisoformat and
TypeAdapter(the corresponding standard type).validate_python, then isoformat
on successful results. No permanent tests or parser implementation were added.

Here, reject means ValueError/TypeError for that specific string input. These
observations do not establish every version or every parsing mode's grammar.

| Input and target | Python stdlib | Pydantic |
|---|---|---|
| Date `20261029` | 2026-10-29 | Reject |
| Date `2026-W44-4` | 2026-10-29 | Reject |
| Date `2026-302` | Reject | Reject |
| Time `09:30` | 09:30:00 | 09:30:00 |
| Time `09:30:00,25` | 09:30:00.250000 | 09:30:00.250000 |
| Time `09:30:00.123456789` | 09:30:00.123456 | 09:30:00.123456 |
| Time `09:30:00+0230` | 09:30:00+02:30 | 09:30:00+02:30 |
| Time `09:30:00+02` | 09:30:00+02:00 | Reject |
| Datetime with one snake emoji replacing T | Accepted, emits T | Reject |
| Datetime `2026-10-29t09:30:00z` | Reject | 2026-10-29T09:30:00+00:00 |
| Datetime with -00:00 | Emits +00:00 | Emits +00:00 |
| Datetime with bracketed Europe/Paris | Reject | Reject |

Python 3.12 tomllib accepted full dates, local times with seconds, local and
offset datetimes, space/lowercase separators, and truncated nine fractional
digits to six. It rejected bare 09:30. Its bare 20261029 value was an integer,
not a date. These are observations of that installed parser, not claims about
TOML 1.1 implementation support.

## Proposed default and remaining decisions

Use the readable extended calendar forms as the default representation, preserving
the agreed first-class tagged literal style and optional explicit offset. Consider
HH:MM, a space separator and lowercase t/z as explicit accepted-input conveniences,
without turning them into different semantic output formats. Keep compact dates,
week/ordinal dates, components and epochs as separate potential extensions rather
than accepting them just because a dependency happens to parse them.

The implementation profile resolves the following questions raised by this research:

- whether omitted seconds mean exactly zero or retain reduced precision;
- fractional precision and behavior at Rust/Python/JSON export boundaries;
- year range, leap seconds and end-of-day 24:00;
- accepted versus emitted separator/offset spellings and preservation rules;
- the distinction between absent offset and the zero-offset conventions;
- whether named-zone/calendar metadata is in scope at all.

The public ISO catalogue identifies ISO 8601-1:2019 and its 2022 amendment, but
full normative text was not available in this research. RFC 3339 Appendix A is
explicitly historical ISO-derived grammar, the W3C document is a 1998 discussion
Note, and EDTF is its own public profile. None justifies claiming exhaustive
modern ISO 8601 support. [ISO catalogue][iso2019], [amendment][iso2022],
[RFC appendix][rfc3339-appendix], [W3C status][w3c], [EDTF][edtf]

[rfc3339-scope]: https://www.rfc-editor.org/rfc/rfc3339#section-1
[rfc3339-grammar]: https://www.rfc-editor.org/rfc/rfc3339#section-5.6
[rfc3339-restrictions]: https://www.rfc-editor.org/rfc/rfc3339#section-5.7
[rfc3339-appendix]: https://www.rfc-editor.org/rfc/rfc3339#appendix-A
[rfc9557-scope]: https://www.rfc-editor.org/rfc/rfc9557#section-1.1
[rfc9557-definitions]: https://www.rfc-editor.org/rfc/rfc9557#section-1.2
[rfc9557-zero]: https://www.rfc-editor.org/rfc/rfc9557#section-2
[rfc9557-zone]: https://www.rfc-editor.org/rfc/rfc9557#section-3.4
[w3c]: https://www.w3.org/TR/NOTE-datetime
[edtf]: https://www.loc.gov/standards/datetime/edtf.html
[iso2019]: https://www.iso.org/standard/70907.html
[iso2022]: https://www.iso.org/standard/81801.html
[toml10]: https://toml.io/en/v1.0.0.html#offset-date-time
[toml11]: https://toml.io/en/v1.1.0#offset-date-time
[python-date]: https://docs.python.org/3.12/library/datetime.html#datetime.date.fromisoformat
[python-time]: https://docs.python.org/3.12/library/datetime.html#datetime.time.fromisoformat
[python-parse]: https://docs.python.org/3.12/library/datetime.html#datetime.datetime.fromisoformat
[python-output]: https://docs.python.org/3.12/library/datetime.html#datetime.datetime.isoformat
[temporal-strings]: https://tc39.es/proposal-temporal/docs/strings.html
[temporal-date]: https://tc39.es/proposal-temporal/docs/plaindate.html#from
[temporal-time]: https://tc39.es/proposal-temporal/docs/plaintime.html#from
[ecma-time]: https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-time-values-and-time-range
[json-formats]: https://json-schema.org/draft/2020-12/json-schema-validation#section-7.3.1
[json-format-vocab]: https://json-schema.org/draft/2020-12/json-schema-validation#format
