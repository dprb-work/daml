# Design principles

DAML is a configuration language with explicit structure, predictable values, readable text, structural contracts, and deliberate defaults. These choices make documents easier to read and validate consistently, but they also make DAML less terse and less interoperable with tools that expect YAML or JSON. Choose it when those tradeoffs fit; it is not a universal replacement for either format.

Start with [data versus schema](data-and-schema.md): one supplies values, the other describes requirements and defaults. Its optional type/value analogy and familiar programming concepts provide a vocabulary for these principles. Follow the [tutorial](tutorial.md) for runnable examples. The [syntax contract](syntax.md), [native authoring contract](native-authoring.md), and [schema completion and validation reference](schemas.md) define exact behavior.

## Make structure visible

Mappings use `{}` and sequences use `[]`. Newlines separate entries, but indentation does not decide which collection owns a value:

```daml
{
  service: {
    endpoint: "https://api.example.test"
  }
}
```

Braces and brackets add visual punctuation, especially in large files. In return, indentation changes cannot silently change which collection owns an entry. Commas and multiple same-line entries are not allowed, which keeps entry boundaries uniform but rules out common JSON-style formatting.

## Keep scalar forms predictable

Strings, numbers, booleans, and `null` have explicit forms. Numbers use JSON decimal syntax; strings use double quotes or DAML's triple-quoted forms. A date-looking token is not silently converted to a date, and a word such as `yes` is not a boolean. The parser does not guess a scalar's type from its spelling.

This reduces parser-dependent surprises and makes values portable as JSON-compatible data. It also means more quoting than YAML, and DAML accepts a narrower set of scalar conveniences. See [scalar and string syntax](syntax.md#scalars-and-input).

## Give multiline text a readable form

Triple double quotes preserve internal layout after a defined dedent. Triple single quotes join wrapped lines with spaces. Both forms end their opening line and place the closing delimiter on its own line, making boundaries visible in a configuration file.

The two forms have distinct rules, so readers must learn which one preserves layout and which one wraps prose. Use ordinary quoted strings for short values. The [triple-string rules](syntax.md#triple-string-boundaries) define boundaries, dedent, and line joining.

## Use structure as the contract

Native definitions describe the shape and constraints of values. A record can remain open to additional fields or use the `closed` modifier when extra fields should fail. Definitions compose through intersections and alternatives; an explicit tag records a definition assertion, while structural constraints still determine whether the value fits.

This lets independently authored definitions constrain the same value without requiring a declared membership graph. It also means a reader must distinguish structural compatibility from record openness. Nominal systems use declared identity or ancestry for compatibility; DAML definition references reuse structural constraints and do not introduce nominal membership. See [structural composition](structural-composition.md) and [native definition expressions](native-authoring.md#composition-records-and-positions).

## Make defaults deliberate

Defaults are explicit data attached to definitions. Completion applies defaults to missing values; supplied values win, and the completed result is then validated. DAML does not invent scalar values or guess which implementation to select.

Defaults reduce repeated configuration and provide a clear baseline. They can also obscure where a value came from if used carelessly, so keep them close to the definition that owns them and document meaningful choices. Exact merge and replacement rules are in [defaults and completion](native-authoring.md#defaults-and-completion).

## Choose with the tradeoffs in view

DAML favors one explicit syntax and one defined interpretation over broad compatibility with YAML conventions. That choice supports predictable configuration and structural validation, at the cost of requiring DAML-specific tooling and more visible punctuation. [DAML and neighboring formats](comparison.md) compares those choices with YAML, JSON, and JSON plus JSON Schema.
