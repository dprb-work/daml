# Parser edge-case evidence

This record separates user-confirmed decisions from parser observations at baseline `79241ba`. The 45 canonical probe inputs and the inspector's raw status, stdout, stderr, and exit code are in [edge-cases.json](edge-cases.json). In this Markdown, `\n`, `\r`, and `\t` inside backticks denote actual LF, CR, and tab bytes; JSON stores each source string with JSON escaping. The inspector prints decoded values using Rust debug escaping. Error wording below is copied from stderr; it is not translated into an inferred `ErrorKind`.

## Confirmed decisions

- Triple-double strings require an opening line ending after the delimiter and optional spaces/tabs. The opening boundary and one line ending before the closing-delimiter line are not content. Additional blank lines are content. Dedent is mandatory.
- Inline triple-double form is invalid. Existing triple-single wrapped-string behavior remains.
- Multiple collection entries require physical newlines. A tag's explicit payload must start on the same physical line. Quoted keys remain rejected. The key/tag alphabet remains provisional.

These are user decisions, not conclusions inferred from current parser output. The triple-string boundary rule is also described in [syntax.md](syntax.md#triple-string-boundaries).

## Observed string cases

The scanner is in `src/strings.rs`; existing focused cases are in `tests/strings.rs`.

| Case | Input | Inspector observation |
|---|---|---|
| `s01` | `"""\n\n  x\n\n"""` | `string "\nx\n"`; additional blank lines survive, while the closing boundary newline is excluded. |
| `s02` | `""" \t\r\n  x\r\n  """` | `string "x"`; opener whitespace is accepted, CRLF boundaries normalize, and common spaces dedent. |
| `s03` | `"""\n\t  x\n \ty\n"""` | `string "\t  x\n \ty"`; tabs do not count toward the space margin. |
| `s04` | `"""\n  x\\\n  y\n"""` | `string "xy"`; continuation removes slash, newline, and next-line indentation. |
| `s05` | `"""\n  x\\ \n  y\n"""` | Rejected: `invalid escape or continuation at bytes 7..10`. |
| `s06` | `"""\n  x \\""" y\n"""` | `string "x \\\"\\\"\\\" y"`; decoded value contains `x ` followed by three literal quotes and ` y`. |
| `s07` | `"""\n  x\n"""suffix` | Rejected: `invalid string syntax at bytes 8..11`. |
| `s08` | `"""\n  x\n  """ #c` | `string "x"`; closing indentation and trailing comment are excluded. |
| `s09` | `"""\n  \n  """` | `string "  "`; all-blank content has zero dedent margin and preserves spaces. |
| `s10` | `"""\n  \t\n    \n  """` | `string "  \t\n    "`; blank-only lines do not set the margin. |
| `s11` | `'''\n \t \n  a \t\n b\n'''` | `string "a b"`; wrapped form trims each line, drops blank lines, and joins with one space. |
| `s12` | `'''\n\n'''` | `string ""`. |
| `s13` | `"""\n\\n remains literal\n"""` | `string "\\\\n remains literal"`; `\\n` remains two literal characters, not a JSON escape. |
| `s14` | `"""\n\tdata\n"""` | `string "\\tdata"`; leading tab remains literal. |
| `s15` | `"""\n        left \\\n    right\n        end\n"""` | `string "    left right\\n    end"`; the continuation line contributes to initial dedent. |
| `s16` | `"""\ntext """ trailing\n"""` | Rejected: `invalid string syntax at bytes 9..12`; an unescaped matching delimiter in content is not an inline terminator. |
| `s17` | `"""\n  """` | Accepted empty string, `@0..9`. |
| `s18` | `"""\n"""` | Accepted empty string, `@0..7`. |

Existing tests also exercise escaped backslash parity and continuation rejection at the closing boundary for LF, CRLF, and CR: `tests/strings.rs:115-162,183-203`.

## Structural and token boundaries observed

Parser token/name rules live in `src/parser.rs` (including `is_name_char`); there is no `src/names.rs` in this checkout. Existing structure tests cover related outcomes (`tests/structure.rs`).

| Case | Input | Inspector observation |
|---|---|---|
| `t01` | `{a.b/c: 1}` | Accepted; mapping key `a.b/c`. |
| `t02`, `t03` | `{é: 1}`, `{a·b: 1}` | Accepted Unicode non-whitespace key characters. |
| `t04` | `{a@: 1}` | Accepted `@` in key; current acceptance is provisional, not an alphabet decision. |
| `t05` | `{a b: 1}` (NBSP between `a` and `b`) | Rejected: `expected ':' after key at bytes 2..4`; Unicode whitespace ends a key token. |
| `t06` | `{a:!x\nb: 2}` | Accepted. `a` is bare tagged empty mapping `!x`; `b` is a separate next entry. The tag does not consume `b` as its payload. |
| `t07` | `{a: !x # c\nb: 2}` | Accepted; `a` is bare tagged empty mapping and `b` is the next entry. Comment after tag name ends that line. |
| `t08` | `[1\n2]` | Accepted two-item sequence. |
| `t09` | `[1 2]` | Rejected: `expected a newline between entries at bytes 3..4`. |
| `t10` | `[1,2]` | Rejected: `expected a newline between entries at bytes 2..3`. |
| `t11` | `{a: 1\na: 2}` | Rejected: `duplicate mapping key at bytes 6..7`. |
| `t12`, `t13` | `null # c`, `null\n# c\n` | Both accepted; trailing comments are trivia. |
| `n01`–`n04` | `01`, `1.`, `1e`, `1e+` | Rejected as invalid decimal numbers; exact stderr is in the JSON record. |
| `n05` | `1x` | Rejected: `invalid decimal number at bytes 0..2`. |
| `n06`, `n07` | `1e2`, `-0` | Accepted with exact numeric lexemes. |
| `n08`, `n09` | `truex`, `nullx` | Rejected: `expected an explicit value at bytes 0..1`. |
| `u01` | UTF-8 BOM followed by `null` at byte zero | Accepted. |
| `u02` | `null\n` followed by BOM | Rejected: `invalid source character at bytes 5..8`; BOM is only permitted at byte zero. |
| `u03` | `"é"` | Accepted decoded `é`. |
| `u04` | `"\uD83D\uDE00"` | Accepted decoded `😀`. |
| `u05` | `'x'` | Rejected: `unexpected syntax token at bytes 0..1`; ordinary single-quoted strings are unsupported. |

## Remaining decisions and verification limits

The final key/tag alphabets, including numeric starts and punctuation, remain
unresolved. Blank-only whitespace and tab behavior above are current observations,
not new formatter-driven language decisions. Any later change needs explicit
input/value/rejection examples and a syntax-contract revision.

The 45 canonical cases cover selected string framing/dedent/continuation cases, tag boundaries, comments, collection separators, provisional names, numbers, BOM, and Unicode. This sample is not exhaustive and does not settle an alphabet.
- The parser accepts `@`, dotted/slashed names, and Unicode letters/punctuation under a broad non-whitespace/nonstructural-character rule. That acceptance is provisional. Formatter preservation cases should not freeze a final identifier policy.
- Bare-tag behavior should be considered in an enclosing context. `t06` and `t07` show a bare tagged empty mapping followed by a separate collection entry; a root tag followed by a second value instead fails the one-root document rule.
- Runtime observations came from `/home/jason/workspace/daml/target/debug/examples/inspect`, freshly built for baseline `79241ba`. The probe created one temporary file per case, wrote each Python string as UTF-8 bytes, invoked the inspector once per file, captured stdout/stderr/exit status, and removed the temporary directory. No build, test, lint, or formatter command was run.

Main replayed all 45 recorded inputs against the shared-trivia parser and matched
their status, stdout, stderr, and byte spans exactly. The 29 accepted cases also
passed the actual formatter at widths 20 and 80: `--check` reported canonical
output and inspection retained values, tags, key/order and numeric spellings.
