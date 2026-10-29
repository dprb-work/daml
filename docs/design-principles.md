# Design principles

A configuration file is something people read, change, review, and get wrong.
DAML's design starts from those activities: make the choices visible, describe
what is required, and turn an incomplete file into a value a program can check.

These choices have costs. DAML asks you to learn a new format and use its tools;
it isn't a universal replacement for YAML or JSON. The [tutorial](tutorial.md)
shows the choices at work. This page explains why they are there.

## Let indentation help the reader, not decide ownership

When you move a field in an editor, changing its indentation shouldn't silently
move it into another object. DAML uses braces and brackets to show ownership:

```daml
{
  service: {
    endpoint: "https://example.com"
  }
}
```

The endpoint belongs to `service` because of the braces, not its column. Newlines
separate entries, and indentation makes the structure easier to scan.

The cost is visible punctuation. DAML also rules out commas and multiple entries
on one line. That is less flexible than JSON-style layout, but gives entries one
consistent boundary rule. See [collection syntax](syntax.md#structure-and-names).

## Make the supplied value mean what it says

If a setting contains a word, it should be clear whether you meant text or a
boolean. DAML uses `true` and `false` for booleans and quotes for strings:
`"yes"` stays text. A date-looking string doesn't silently become a date.

This removes implicit scalar interpretations, at the cost of more quoting than
YAML and fewer shorthand forms. Numbers follow JSON decimal syntax; detailed
constraints belong to definitions rather than guessed scalar meanings. See
[values and strings](syntax.md#scalars-and-input).

## Let text look like the text you are writing

A message and a template have different needs. A paragraph should be free to
wrap in the editor; a template may depend on its line breaks and indentation.
DAML gives them distinct multiline forms:

- Triple-single-quoted text joins wrapped prose into one logical line.
- Triple-double-quoted text preserves layout after a defined dedent.

You have to learn the distinction, and neither form replaces ordinary quoted
strings for short values. The [text example](../examples/tutorial/data/text.daml)
shows both; the [string reference](syntax.md#triple-string-boundaries) owns the
exact delimiter, escape, dedent, and joining rules.

## Ask for a contract instead of a catalogue

Suppose a service needs an adapter with a string endpoint. It can ask for that
shape rather than name every provider it will ever accept. A separately authored
provider can add its own settings and defaults while still meeting the shared
requirement.

That is why structural composition matters more than syntax alone. You still
need to make a provider's bundle available and choose it in the data, but adding
one needn't mean rewriting the consumer's contract.

The cost is a distinction worth learning: *fits the required shape* isn't the
same as *may have extra fields*. A closed provider record can satisfy an open
adapter contract. [Structural composition](structural-composition.md) explains
that with the working examples, including alternatives and nominal typing.

## Let defaults shorten a file without hiding bad choices

Most workers might use three retries. Put that baseline in the definition and
let each configuration supply a different integer when it needs one. A supplied
string such as `"many"` should fail, not disappear behind the default.

When you ask DAML to resolve a configuration, it completes missing values and
validates the result. Defaults can be composed, but they are not an arbitrary
merge convention: different valid completions can be ambiguous rather than an
excuse to pick one silently. The result also retains input/default origins so a
consumer can inspect where values came from.

Keep meaningful defaults near the definition that owns them. They reduce
repetition, but a reader can still miss important behavior hidden behind too
many layers. Start with [data and schema](data-and-schema.md), then use
[the default rules](native-authoring.md#defaults-and-completion) when you need
the precise precedence and replacement behavior.

## Prefer an honest fit over a universal-format claim

Choose DAML when readable configuration, constraints, defaults, and independently
developed components belong in one workflow. JSON remains a strong choice for
ordinary interchange. YAML's ecosystem can matter more than DAML's narrower,
more explicit syntax.

DAML is not YAML-compatible, doesn't support every arbitrary JSON object key,
and needs its own tooling. The [comparison](comparison.md) weighs those tradeoffs.
The [syntax](syntax.md), [native definitions](native-authoring.md), and
[resolution reference](schemas.md) remain the authority for exact behavior.
