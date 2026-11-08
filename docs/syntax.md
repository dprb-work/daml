# DAML syntax contract

This reference defines data and native-definition syntax. [Schema completion
and validation](schemas.md) are separate passes. The [formatter](formatting.md)
supports both grammars; the [language server](lsp.md) provides native syntax
diagnostics and formatting only. Data parsing never infers nesting, string
types, or missing values and does not fall back to YAML. See the [native
authoring contract](native-authoring.md) for definition expressions and
bundles.

## Structure and names

A document contains one nonempty root value. Mappings use `{}` and sequences use
`[]`. Entries are separated by physical newlines outside strings/comments; an
entry may instead be followed directly by its collection's closing delimiter.
Single-entry inline collections are valid. Commas and multiple same-line entries
are errors. Spaces/tabs, blank lines, and `#` line comments are presentation.
Indentation never establishes collection ownership.

Mapping keys are nonempty, unquoted tokens followed by `:`. Values are explicit
strings, numbers, `true`, `false`, `null`, mappings, or sequences. Keys have no
escape syntax and repeated exact key spellings are errors. Dots and slashes in
keys are ordinary characters; they do not imply nested lookup, paths, or file
access.

Keys use Unicode 16 letters and numbers, followed by letters, numbers, combining
marks, `_`, `-`, `.`, or `/`. Letters, numbers, `_`, `-`, `.`, and `/` may start
a key, so digit-leading keys are valid. A combining mark may only follow another
character. Tags use the same letters, numbers, and subsequent combining marks,
with `_` and `-` permitted; dots separate nonempty tag segments, and slashes are
not permitted. Tag segments cannot start with a combining mark. No Unicode
normalization or case folding is performed.

Both parsers use these name rules. Native definition syntax adds constraint and
default expressions; successful syntax parsing does not imply that a constructor,
metadata field, or bundle reference is semantically valid.

Positional constraints require native `!Tuple [ ... ]` syntax. Bare brackets
remain ordinary sequence data; they do not declare tuple positions. The native
authoring contract owns tuple default placement and the separate `!List` facets.

One optional `!tag` precedes any value. Tag names are literal and unknown tags
are preserved rather than resolved by parsing. Stacked tags are errors.
An explicit payload starts on the same physical line as the tag. If a newline,
comment, closing delimiter, or EOF follows the name, the value is an empty tagged
mapping. A subsequent field never becomes its payload by indentation:

```daml
{
  adapter: !network.Adapter
    timeout: 30
}
```

Here `timeout` belongs to the outer mapping. To put it inside the adapter, write
`adapter: !network.Adapter { timeout: 30 }`. Braces are authoritative; the
formatter presents that structure consistently.

## Scalars and input

Numbers use the JSON decimal spelling: optional minus; zero or a nonzero digit
followed by digits; optional fraction with digits; optional exponent with digits
and an optional sign. Leading zeroes, incomplete fractions/exponents, hexadecimal,
NaN, Infinity, and implicit dates are errors. Numeric lexemes retain their exact
source bytes: parsing never rounds an integer through `f64` or silently limits
it to `i64`. Type/range assertions belong to subsequent validation/conversion.

Input is UTF-8 (`str`/`String` at the Rust boundary). LF, CRLF, and CR separate
lines; content line endings in triple strings normalize to LF. One leading UTF-8
BOM is permitted at byte zero. Empty/comment-only documents, trailing values,
YAML document markers/directives, anchors, aliases, block collections, `|`/`>`
scalars, and ordinary single-quoted strings are not accepted.

`#`, braces, tag markers, and other syntax characters inside strings are data.
String contents have no interpolation or environment substitution; the escape
and continuation rules below define their decoding.

## Ordinary double-quoted strings

`"..."` uses JSON escapes: quote, backslash, slash, backspace, form feed, newline,
carriage return, tab, and four-hex-digit Unicode escapes. Valid UTF-16 surrogate
pairs decode to their scalar; lone or malformed surrogates fail. Unknown or
truncated escapes and raw ASCII control characters, including newlines/tabs,
are errors. Use a triple form for source text spanning physical lines.

Unescaped ordinary content references the original source without a second
string allocation. Escaped content owns its decoded text.

## Temporal literals

`!Date`, `!Time`, and `!DateTime` construct typed values in data documents:

```daml
!Object {
  day: !Date "2026-10-29"
  starts: !Time "09:30"
  deadline: !DateTime "2026-10-29T10:30:00.123456789123Z"
}
```

The constructor names are reserved. A generic assertion may precede one
constructor, as in `!app.EventDate !Date "2026-10-29"`. Arbitrary assertion
stacking remains invalid. A date-looking quoted string without a constructor
is still an ordinary string; parsing never guesses temporal types.

The accepted profile is an ISO 8601-1:2019 extended-calendar subset, informed by
the relevant 2022 amendment. It is not a claim of full ISO 8601 conformance.
Dates use Gregorian `YYYY-MM-DD` with four year digits, optionally preceded by
`-` for a nonzero year before year zero. The supported range is -9999 through
9999, including year zero. Month lengths and Gregorian leap years are checked.

Times use `HH`, `HH:MM`, or `HH:MM:SS`. Standalone `Time` also accepts an
optional leading uppercase `T`. `DateTime` joins a complete date and clock with
uppercase `T`. An optional fraction follows seconds only, uses `.` or `,`, and
must contain ASCII digits. Lower-order omitted clock fields become zero;
fractions on hours/minutes and basic/compact clock spellings are unsupported.

Time and DateTime may end with uppercase `Z` or signed `HH`/`HH:MM` offsets.
Offset fields are bounded by 23 hours and 59 minutes; negative zero is rejected.
No offset stays absent, never local or UTC by inference. Canonical values use
full clock fields, a dot fraction, uppercase `T`/`Z`, and `±HH:MM`; zero offsets
write as `Z`. Every supplied fractional digit is retained.

Hour 24 is accepted only when lower fields and fraction are zero. It remains
`24:00:00` on the authored date; it does not roll over. Second 60 is preserved
as a leap-second label, but DAML does not check whether it occurred on a
particular date. With an explicit offset, the corresponding UTC minute must be
23:59. DAML does not consult IERS data, a clock, or a network service.

Temporal values are distinct from strings and from each other. Native
`!String` does not accept a temporal value. A native `!Date`, `!Time`, or
`!DateTime` constraint checks the corresponding intrinsic kind; Time and
DateTime can additionally require or forbid an explicit offset. See
[native temporal constraints](native-authoring.md#temporal-values).

JSON has no temporal primitive. Plain JSON export writes canonical ISO text and
loses the intrinsic kind. Semantic representations retain a `date`, `time`, or
`datetime` discriminator with canonical text.

## Native Regex literals

Native definitions add one typed scalar that ordinary data documents do not:

```daml
pattern: !Regex "\d{4}-\d{2}-\d{2}"
```

The double quotes delimit the Regex body. Backslashes and other characters
inside remain raw source for Rust `regex`; ordinary DAML string escapes are not
decoded. The body must stay on one physical line and cannot contain a double
quote; use the regex escape `\x22` when matching one. The body remains subject
to the scalar-size limit. The formatter preserves its contents exactly.
Semantic compilation, full-string matching, and
the valid position under `String.pattern` belong to the
[native authoring contract](native-authoring.md).

## Triple string boundaries

Both triple forms start with a delimiter followed only by spaces/tabs and a line
ending. The closing delimiter occupies its own physical line, with indentation
before it and only spaces/tabs or a comment after it. A matching unescaped triple
delimiter elsewhere is an error, not an alternate inline form.

The opening line ending and one line ending immediately before the closing
line are syntax, not content. Closing-line indentation is not content. Extra
blank lines inside literal strings remain data; there is no chomping suffix.

`\\` produces a literal backslash in either triple form. A backslash immediately
before that form's matching triple delimiter makes the delimiter literal. Other
backslashes remain data, except the continuation rule for triple-double strings.
For example, a literal `\n` remains those two characters in triple text; it is not
an ordinary-string newline escape.

### Triple double quotes: literal, mandatory dedent

`"""..."""` preserves internal layout after this exact sequence:

1. Exclude delimiter-boundary line endings as above.
2. If at least one physical content line is nonblank, compute the minimum leading
   **space** count among nonblank lines. Tabs remain literal data and never count
   as indentation columns.
3. Remove that common space prefix. Remove up to the same prefix from blank
   lines, preserving remaining data and the number of internal line breaks.
4. If the complete body is blank, instead remove all spaces/tabs from every
   content line while preserving the number of internal line breaks.
5. Decode escaped backslashes/delimiters and process explicit continuations.

The closing delimiter's column and enclosing node depth do not set the margin.
Moving equivalent content deeper into a map/list must preserve its decoded
bytes. Additional indentation within the text remains meaningful.

An unescaped backslash directly before an internal line ending removes itself,
the line ending, and the next line's leading spaces/tabs. Joining inserts no
space: a space before the backslash stays data. A backslash followed by spaces/
tabs before the newline is an error. Escaped backslashes do not continue lines.
A dangling continuation at the excluded closing boundary is an error.

Dedent precedes joining because a continuation can consume the physical line
that establishes the minimum margin. It must not change that margin retroactively.

### Triple single quotes: wrapped single-line text

`'''...'''` trims spaces/tabs from each physical content line, discards blank
lines, and joins remaining lines with one space. It has no continuation rule;
backslash-newline does not acquire extra meaning. Escaped backslashes/matching
delimiters follow the shared triple rules. Internal whitespace within each
trimmed line is unchanged.

## Source model and errors

`parse` returns an owned `Document`; input `String` ownership can be transferred
without a source copy. Keys, tags, numeric lexemes, and unchanged strings use
byte spans into that source. Changed string values own decoded text. Nodes retain
their original delimiter/tag-inclusive span, not positions in transformed text.
The complete source, including comments, stays available for later tooling.

Errors have a structured category and a half-open original byte span. Parsing
fails at the first error and consumes the complete root on success. Checked
source slicing returns `None` for out-of-bounds or non-character-boundary spans.

Default bounds are 8 MiB input, 1 MiB per scalar token/key/tag, 100,000 value
nodes, and 128 nested mappings/sequences. Tags do not add wrapper nodes. Custom
limits may lower these bounds or raise size/node limits, but nesting above the
hard ceiling of 128 is rejected. Exhausting a bound returns a structured error,
not a stack overflow or a fabricated partial document.

## Scope and source reuse

The small native parser implements this contract directly. Upstream YAML/KYAML
and Rust parser sources were consulted through the [reuse assessment](development/parser-reuse.md),
but this syntax implementation does not copy or vendor upstream code or tests
and needs no YAML parser dependency. YAML's full acceptance corpus is not the
DAML grammar.
Permanent cases cover consumer-visible syntax, values, original spans, failure
categories, limits, and the current key/tag alphabets.
