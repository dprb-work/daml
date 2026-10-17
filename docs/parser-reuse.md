# Parser reuse assessment

Evidence reviewed on 2026-10-17. This is a source assessment, not an executed
parser comparison, license opinion, dependency approval, or code import.
[The implementation plan](../PLAN.md) owns the proposed work and acceptance gates.

## What YAML and KYAML actually provide

KYAML provides useful explicit `{}`/`[]` structure, quoted string values,
comments, and formatting examples. It is primarily an output specification and
remains a YAML subset. The upstream Go package is an encoder, not a configurable
DAML parser. Its source still uses a YAML decoder for incoming data. [KEP-5295](https://github.com/kubernetes/enhancements/blob/5968cde694b5e31807440cf36c196a936f9b9413/keps/sig-cli/5295-kyaml/README.md#proposal), [encoder source](https://github.com/kubernetes-sigs/yaml/blob/aadfabb7d36952151672ec164d4220a6f0e31f63/kyaml/kyaml.go)

DAML's comma-free collections and triple-quoted strings are not YAML flow
syntax. A bare tag with absent content is also different: YAML permits tagged
empty scalars; DAML wants that spelling to mean an empty tagged mapping.
A stock YAML parser followed by a shape validator cannot supply these grammar
changes. [YAML flow collections](https://yaml.org/spec/1.2.2/#74-flow-collection-styles), [flow scalars](https://yaml.org/spec/1.2.2/#73-flow-scalar-styles), [flow nodes](https://yaml.org/spec/1.2.2/#75-flow-nodes)

DAML also requires unquoted, spaceless keys; unlike YAML/KYAML, it has no
quoted-key fallback. The proposed identifier character set is a separate DAML
decision recorded in the plan. An upstream parser accepting quoted keys cannot
serve unchanged as DAML's syntax authority.

Reuse YAML's separation of syntax, tag resolution, and construction as a design
lesson. Parsing a tag need not recognize its schema or construct an application
object. YAML's presentation model also distinguishes comments and scalar style
from values. DAML's actual assertions and source-preservation contract still
need their own definition. [Processes and models](https://yaml.org/spec/1.2.2/#chapter-3-processes-and-models)

## Candidate assessment

| Candidate | Relevant source facts | Proposed use |
|---|---|---|
| Saphyr / saphyr-parser, `b8205d5f60ea44f757f7d075fe4f3704bbe5dccc` | Low-level events carry tags; spanned receivers exist. Scanner/parser implement YAML flow commas, block scalars, anchors, directives, and YAML quoting. Direct parser dependencies include `arraydeque` and `thiserror`. | Strongest Rust reference. Inspect small reusable pieces and tests; do not assume a grammar switch exists or import the complete YAML state machine. |
| yaml-rust2, `e12069447073e0c8dd9ceab415159c4c92927534` | README promises basic maintenance/stable API and directs new work to Saphyr. Its parser has the same grammar mismatch; receiver markers are not a complete DAML start/end-span contract. | Historical reference, not the preferred new fork. |
| Winnow, `bca21ee8e3c9980863d18740cbcf2943f787daf2` | Parser combinators and locating/span facilities, not a YAML/DAML grammar. Using it still requires designing DAML syntax and errors. | Viable helper dependency if a focused trial demonstrates simpler code and equivalent diagnostics. Not rejected merely because the crate is currently dependency-free. |
| Kubernetes KYAML encoder, `aadfabb7d36952151672ec164d4220a6f0e31f63` | Go output implementation with file-level Apache-2.0 notice; multiline strings use YAML flow folding. | Format/test inspiration, or explicitly attributed ports of useful isolated logic; not the Rust parser core. |

Primary candidate sources:

- Saphyr [README](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/README.md), [scanner](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/src/scanner.rs), [parser](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/src/parser.rs), [manifest](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/Cargo.toml).
- yaml-rust2 [README](https://github.com/Ethiraric/yaml-rust2/blob/e12069447073e0c8dd9ceab415159c4c92927534/README.md), [parser](https://github.com/Ethiraric/yaml-rust2/blob/e12069447073e0c8dd9ceab415159c4c92927534/src/parser.rs).
- Winnow [manifest](https://github.com/winnow-rs/winnow/blob/bca21ee8e3c9980863d18740cbcf2943f787daf2/Cargo.toml), [locating input](https://github.com/winnow-rs/winnow/blob/bca21ee8e3c9980863d18740cbcf2943f787daf2/src/stream/locating.rs), [span API](https://github.com/winnow-rs/winnow/blob/bca21ee8e3c9980863d18740cbcf2943f787daf2/src/parser.rs).

**Recommendation:** start with a small DAML-native scanner and direct parser,
selectively reusing suitable upstream material. Before committing to a broad
fork or parser framework, check its seam against the same difficult DAML
fixtures. Do not implement two complete parsers to make this decision. No timing,
allocation, or implementation-effort comparison was executed for this note.

DAML's `unsafe_code = "forbid"` applies to DAML's crate, not automatically to
transitive dependencies. A new dependency needs its own source, features,
maintenance, license, and safety review; this plan does not invent a blanket ban
on dependencies containing unsafe code.

## String and source-position traps

Saphyr's quoted scanner performs YAML folding; its block scanner includes YAML
indent/chomping state. Neither is an unchanged implementation of DAML's
triple-double string. Study the algorithms, but preserve DAML's own contract.
[Quoted/block scanner source](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/src/scanner.rs)

DAML dedents nonblank physical content lines **before** continuation joining.
Joining first can erase the least-indented line and change the baseline. The
baseline must not depend on the enclosing collection or closing delimiter's
column. Test source byte spans independently of decoded string positions.

Do not accept a span API's name as evidence of UTF-8 byte fidelity. Saphyr exposes
markers/spans, but its inspected span tests also use character iteration to
extract ranges. Require fixtures with non-ASCII text before and inside a node
before adapting position logic. This is a verification requirement, not a claim
of an upstream bug. [Span tests](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/tests/span.rs)

## Reusing upstream tests

Use selected YAML Test Suite cases to probe shared Unicode, escape, key,
collection, and tag behavior, not to claim full YAML compatibility. Every
selected case needs a DAML expectation: unchanged, adapted, or intentionally
rejected. Preserve the original case ID and record adaptations.
Quoted-key and whitespace-containing-key cases must be adapted to DAML's
unquoted-key contract or retained as explicit rejection cases, not counted as
shared positive syntax.

Use a dated data release, not the moving `data` branch: upstream warns that the
latter can be incorrect and is force-pushed. A concrete available pair is:

- Data tag `data-2022-01-17`, commit `6e6c296ae9c9d2d5c4134b4b64d01b29ac19ff6f`.
- Corresponding source tag `v2022-01-17`, commit `45db50aecf9b1520f8258938c88f396e96f30831`.

[Tag metadata](https://api.github.com/repos/yaml/yaml-test-suite/tags?per_page=10), [usage guidance](https://github.com/yaml/yaml-test-suite/blob/da267a5c4782e7361e82889e76c0dc7df0e1e870/ReadMe.md#usage), [tag vocabulary](https://github.com/yaml/yaml-test-suite/blob/da267a5c4782e7361e82889e76c0dc7df0e1e870/doc/tag-usage.md)

Do not translate arbitrary YAML to DAML text with regex preprocessing and then
call an existing YAML parser. String/comment boundaries, source spans, and
error locations would require another real parser, while unsupported YAML could
leak through.

## Copying, porting, and dependency obligations

The user permits reuse with an appropriate license. Treat copied/ported routines
and fixtures as derived material; changing language or identifiers does not make
provenance disappear.

- Saphyr's license identifies original Chen Yuheng code and Ethiraric changes;
  redistribution requires both authors' notices/licenses. Each lineage offers
  MIT or Apache-2.0. Preserve the applicable selected terms for both lineages,
  rather than retaining only a package's SPDX label. [License](https://github.com/saphyr-rs/saphyr/blob/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/LICENSE), [license texts](https://github.com/saphyr-rs/saphyr/tree/b8205d5f60ea44f757f7d075fe4f3704bbe5dccc/parser/.licenses)
- yaml-rust2's MIT/Apache alternatives include both authors. [MIT](https://github.com/Ethiraric/yaml-rust2/blob/e12069447073e0c8dd9ceab415159c4c92927534/MIT-LICENSE), [Apache](https://github.com/Ethiraric/yaml-rust2/blob/e12069447073e0c8dd9ceab415159c4c92927534/Apache-LICENSE)
- Winnow has MIT terms and a contributor/nom-lineage copyright record. Inspect
  both for copied material. [License](https://github.com/winnow-rs/winnow/blob/bca21ee8e3c9980863d18740cbcf2943f787daf2/LICENSE-MIT), [copyright](https://github.com/winnow-rs/winnow/blob/bca21ee8e3c9980863d18740cbcf2943f787daf2/COPYRIGHT)
- KYAML's encoder file carries Apache-2.0 terms. The broader repository has other
  licensing lineage; do not apply one file's header to every file in the repo.
  [Encoder](https://github.com/kubernetes-sigs/yaml/blob/aadfabb7d36952151672ec164d4220a6f0e31f63/kyaml/kyaml.go), [repository license](https://github.com/kubernetes-sigs/yaml/blob/aadfabb7d36952151672ec164d4220a6f0e31f63/LICENSE)
- YAML Test Suite uses MIT, copyright 2016–2020 Ingy döt Net. Include its copyright
  and permission notice with copied fixtures. The data tag did not expose a
  root `License` at the inspected URL; retain the matching source tag's license
  explicitly. [Matching license](https://github.com/yaml/yaml-test-suite/blob/45db50aecf9b1520f8258938c88f396e96f30831/License)

Before importing anything, record repository, exact revision, copied paths,
local destinations, modifications, and license/notice files. Track deliberate
updates, not automatic upstream syncing. A Cargo dependency additionally needs
an inventory of resolved dependency licenses. DAML's own distribution license
is not selected by this research; `publish = false` remains in place.

YAML's specification itself says it may be copied if unmodified. Link and
independently describe its semantics rather than copying and editing the spec
as if it had the test suite's MIT terms. [Specification notice](https://yaml.org/spec/1.2.2/)
