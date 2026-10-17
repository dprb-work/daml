# DAML syntax contract

This contract owns the first syntax-to-document implementation (plan milestones
1–2). Schema validation, formatting, and LSP integration remain subsequent
milestones. The governing rule is **one parse, no guessing**: no implicit nesting,
implicit string typing, missing-value defaults, or YAML fallback.

## Structure and names

A document contains one nonempty root value. Mappings use `{}` and sequences use
`[]`. Entries are separated by physical newlines outside strings/comments; an
entry may instead be followed directly by its collection's closing delimiter.
Single-entry inline collections are valid. Commas and multiple same-line entries
are errors. Spaces/tabs, blank lines, and `#` line comments are presentation.
Indentation never establishes collection ownership.

Mapping keys are nonempty, unquoted tokens followed by `:`. Values are explicit
strings, numbers, `true`, `false`, `null`, mappings, or sequences. Keys have no
escape syntax and repeated exact key spellings are errors. Dots/slashes do not
imply nested lookup, paths, or file access.

The final key and tag character alphabets, including whether they align, are
intentionally deferred. For now both accept non-whitespace characters except
structural punctuation: braces, brackets, colon, comma, hash, quotes, exclamation,
backslash, ampersand, asterisk, pipe, greater-than, and percent. Control characters
are invalid. This bounds tokens without prematurely deciding identifier rules;
acceptance of numeric-looking, dotted, slashed, or Unicode names is provisional.
There is no runtime key/tag grammar mode.

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
`adapter: !network.Adapter { timeout: 30 }`. Braces are authoritative; the future
formatter will present that structure consistently.

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
There is no interpolation, include mechanism, environment lookup, schema import,
or application execution during parsing.

## Ordinary double-quoted strings

`"..."` uses JSON escapes: quote, backslash, slash, backspace, form feed, newline,
carriage return, tab, and four-hex-digit Unicode escapes. Valid UTF-16 surrogate
pairs decode to their scalar; lone or malformed surrogates fail. Unknown or
truncated escapes and raw ASCII control characters, including newlines/tabs,
are errors. Use a triple form for source text spanning physical lines.

Unescaped ordinary content references the original source without a second
string allocation. Escaped content owns its decoded text.

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
2. Compute the minimum leading **space** count among nonblank physical lines.
   Tabs are literal data, not indentation columns. Blank lines do not lower the
   baseline; all-blank content has a zero baseline.
3. Remove that common space prefix. Remove up to the same prefix from blank
   lines, preserving remaining data and the number of internal line breaks.
4. Decode escaped backslashes/delimiters and process explicit continuations.

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
There are no source references into sibling owned fields, and parsing does not
construct application types based on tags.

Default bounds are 8 MiB input, 1 MiB per scalar token/key/tag, 100,000 value
nodes, and 128 nested mappings/sequences. Tags do not add wrapper nodes. Custom
limits may lower these bounds or raise size/node limits, but nesting above the
hard ceiling of 128 is rejected. Exhausting a bound returns a structured error,
not a stack overflow or a fabricated partial document.

## Scope and source reuse

The small native parser implements this contract directly. Upstream YAML/KYAML
and Rust parser sources were consulted through the [reuse assessment](parser-reuse.md),
but this implementation does not copy or vendor upstream code or tests and adds
no third-party dependency. YAML's full acceptance corpus is not the DAML grammar.
Permanent cases cover consumer-visible syntax, values, original spans, failure
categories, and limits; final key/tag alphabet restrictions are not pinned.
