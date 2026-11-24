# Conventions

These are the general code and style rules shared by this family of repositories. Every repository carries an identical copy of this generated file, kept in step by a drift check: do not edit a copy; change the upstream rules instead. A repository's own instructions (its `AGENTS.md`) take precedence where they differ, and a language section applies only where that language is used.

## General

- Use American English spelling in identifiers, comments, strings, test names,
  and commit subjects. The cancel family is the one exception: write
  `cancelled` and `cancelling` with two l, as Python does. Rename existing
  public names only deliberately, as a listed change.
- For all code, put comments on their own line above the explained line, never trailing on it.
  Only tool-required same-line markers (linter/Sonar suppressions or type-checker directives) are exceptions.
- When the owner declares a Sonar finding a false positive, disable it in code on the reported line with the rule ID and a short reason; never mark it on the server or rewrite working code.
- Keep ownership explicit, data flow direct and side effects local. Thin
  orchestration routes work; domain modules own policy. One question (what a
  function is called, where a value comes from) has one owner, with no fallback
  ladder beside it. Control flows through plain functions with explicit
  arguments, not through nested closures and callbacks or a mutable record
  threaded through a traversal.
- Domain types and constants follow their concept: keep module-local values local; a provider owns what its consumer imports; independent peers share one agreement-named module; truly cross-cutting values with no single owner may use central `constants.py`; host/deployment-dependent values are settings with named defaults beside them.
  Name every meaningful number; keep a shared constant with the type that gives it meaning. Prefer an enum member, literal type or field default on that type; use a loose upper-case constant only for a scalar with no type to belong to.
- Prefer the project's own form over the corresponding standard-library form
  when the project has one.
- Change only what the task needs. Do not add unrequested "improvements" to
  working code; propose them separately.
- Choose the design that solves the actual problem correctly, then prefer the
  smallest such design. A large change is right when the problem is large or the
  existing model is wrong and must be replaced. A change that grows by accumulating
  special cases, guards or normalization around a model you kept is a sign the
  model is wrong: stop and say so before building it. When your change is much
  larger than the request suggests, state why at the start, not in the PR description.
- Before adding structure, show why the current owner cannot carry the need.
  Reuse the existing mechanism and spelling; a new type is fine. A new mechanism
  beside an existing one (a registry, flag, ledger, cache or walker, a second
  spelling of an existing construct, or a copy of an existing mechanism), and a
  new parameter, flag or public surface, need a justification stated in the
  change: why the existing mechanism or surface cannot carry the need. A surface
  that the project's users write against (authoring arguments, configuration
  grammar, schemas) also needs the owner's agreement. Without that justification
  it is a finding. Green tests and speculative reuse do not justify layers.
- Validate incoming shape and representation at the owning boundary, then use
  admitted values directly. Classify a value once, at that boundary, into a
  closed type; nothing below it tests types again. A long `if` chain or inline
  type checks followed by `raise` mean the code works around loose typing.
  Recheck only distinct invariants invalidated by mutation, staleness,
  authorization, concurrency or a new trust boundary. Types alone are not
  runtime proof; small parsers are not defects by shape.
- Signatures say what the code needs. A required value is not optional (no
  `None` default that is then refused). An operation has one entry point, not a
  second one that only adds a return value. A result is one value or a named
  record, not a tuple with positional meaning. An object is complete when one
  builder constructs it; nothing sets its state afterwards.
- Do not put code inside other code (a script in YAML, a program assembled in a
  string of another language). Where it is unavoidable, it lives in its own file
  that is linted and imported.
- Fail fast, in the earliest phase that has the information to establish the
  violation, before dependent effects, and report the real cause. Do not hide
  broken contracts behind defaults or redundant internal checks. Propagate
  unexpected errors with their cause; catch only for recovery or boundary
  translation. Preserve cleanup, cancellation and atomicity. Public errors use
  public exception names.
- A new exception type only where a caller handles that failure differently;
  otherwise use the existing type. Where one type covers several causes that a
  test or an operator must tell apart, the exception carries a reason from a
  closed enum. Message text is for people: nothing matches it, neither tests nor
  code. Tests name the expected failure by its type and, where present, its
  reason (or the refused field carried as data); where neither exists yet, pair
  the refusal with a control that differs in one input and is accepted.
- Run each step once, in the phase that has its information, and pass its
  result on. Do not run a step early and again later (prepare, edit, prepare),
  compute a value twice and make the two agree, or make repeats harmless with
  guards.
- Which failures earn code. A review finding earns code only if the failure can
  happen on a shipped path AND its consequence is a wrong result, lost data, a
  security gap or a hang in normal operation; everything else is recorded as
  known and not built; findings rated Info or Low get no follow-up PR by default.
  The same test applies to a new guard, handler, re-raise or wrapper: no guard
  for a case no shipped path has, or one an existing check already covers; a
  failure during a failure, or a re-raise for a clearer message, earns its
  complexity only when it prevents a serious issue, makes a failure much easier
  to trace, or covers one that happens often. Catch deep throws far up, at a
  boundary that can act, instead of adding a handler at each level.
- Add compatibility only for a concrete external consumer, persisted-data,
  migration or rollout need, not merely because cleanup breaks old code.
- Use repository-owned tools and required gates. Tests assert behavior at a
  public boundary and name failures as the exception rule above says; a test
  that restates the implementation proves nothing. Do not use debug-only
  assertions for live validation unless their execution is guaranteed. Tooling
  selects files from one map, not from hand-kept lists of file names.
- Keep one pull request readable in one sitting: about 500 production lines is
  the upper bound, tests not counted. Split larger work into slices that can be
  reviewed on their own. Slices never wait on each other: each starts from the
  repository's release or main branch and can merge on its own. Never stack a
  branch on an unmerged pull request; a change that truly needs another
  unmerged change is blocked, not stacked.
- Small adjustments may share a pull request. Up to five minor, unrelated
  changes may go into one pull request, or ride on an open pull request that
  has not yet been reviewed, when each is something the owner would skim and
  approve: documentation wording, a rename, a spelling fix, an explanatory or
  analyzer-disable comment, a default value. Each adjustment is one line in the
  description, so it can be read and rejected alone; one that the owner or a
  reviewer questions is taken out rather than holding the rest. Not eligible: a
  feature; a behavior change that needs its own tests-first round; anything
  touching authorization, credentials, concurrency or the CI gate; and any
  bundle above about 100 changed production lines. Those keep their own pull
  request.
- The project owns how a pull request description is built: its conventions
  and, where present, its `.github/pull_request_template.md` define it. Start
  every description from that template; `gh pr create --body-file` skips it, so
  copy it into the body file rather than writing a new form or reusing blocks
  of older pull requests.

## Python

- Prefer functional, data-oriented design when it fits the domain and framework. Start with small functions, explicit inputs and outputs, closures that capture data (not control flow), generators, comprehensions, `functools`, `itertools`, and `contextlib` lifecycle helpers before adding private behavior classes. A class needs a reason a reader can name: identity, polymorphism, a protocol, long-lived owned state, or a framework contract.
- Do not subclass `dict`, `list` or `set` to attach behavior, and do not hide work (locks, I/O, construction) behind `__missing__`, `__getitem__` or a property. A cache is a function plus the project's cache helper.
- Use one typed record per shape. Decode once into a record or a tagged union, then branch on the type with one exhaustive `match`; below the decoder, no `isinstance` check ends in `raise`. Do not pass `dict[str, Any]` between internal functions, build internal records as dict literals, or choose behavior by string comparison or string keys (`.get("kind") == "…"`, `x["key"]`); string keys belong at the decode boundary.
- A closed set of named values that code compares or branches on (a status, a state, a kind, a reason) is an enum (by default `StrEnum` or `Enum`); code compares against its members, not string literals. `Literal` is for a small option set that is only passed through to one consumer.
- Order new/touched Python modules: docstring; imports; one constants block; type aliases/type variables; domain types (enums, structs, protocols; data before behavior); public functions and classes with behavior; private helpers. Functions go top-down: entry points, then helpers in first-use order.
  Constants: public before private, related values adjacent, derived after inputs, units in names (`_SECONDS`, `_BYTES`), import-time values only. Never scatter constants or mass-reorder existing modules; reorder-only work is a separate deliberate PR.
- Declare value constraints on the type (for example `type Positive = Annotated[int, Meta(ge=1)]`) and let the decoder refuse bad values. Hand-write only cross-field rules, once, at the owner; do not re-check downstream.
- Name a condition with more than two `and`/`or` terms as a predicate, or let the type carry it.
- Declare type aliases with the `type` statement; do not use `NewType`. Do not add a `TypedDict` where the project has a record type. Do not `cast` to quiet the type checker: fix the type that forces it, or state the reason at the line.
- Do not import an underscore name from another module: it is that module's interface (drop the underscore), or the importing code belongs in that module.
- `pytest.raises` names the exception type; then assert the reason or the carried field, never `match=` on prose (the language-neutral exception rule).
- For lifecycle or resource code, prefer `contextmanager`, `asynccontextmanager`, `ExitStack`, `AsyncExitStack`, or a cleanup callable over an ad hoc manager class.
- For evolving state, prefer a transition function such as `next_state = step(state, event)` over mutating `self` when object identity is not the abstraction.
- Prefer explicit pytest marks on each test instead of module-wide `pytestmark`.
- Request pytest fixtures by naming them as test-function parameters, even when only their side effects are needed. Do not use `@pytest.mark.usefixtures`.
- For inline test patches, use `monkeypatch` (or the test's existing seam).
- In Ruff-managed repositories, prefer `ruff check --fix` before manual cleanup of autofixable issues.
- Use concise Google-style docstrings when documentation is useful.

## TypeScript

- Prefer `type` aliases by default. Use `interface` when declaration merging, `implements`, or intentional external extension calls for it.
- Prefer literal unions or `as const` objects over `enum`, except where interop or a project-local convention calls for `enum`.
- Prefer named `function` declarations for top-level operations and exported APIs; use `const` arrow functions for callbacks, handlers, predicates, and other function values.
- Prefer named exports for reusable modules. Use a default export for framework-required entrypoints or where the project convention favors it.

## React

- Prefer `function ComponentName(...)` for exported components. Avoid `React.FC` unless the project has standardized on it; use explicit prop types.
- Avoid `useMemo` and `useCallback` by default. Use them when measurement or a clear identity/performance issue justifies the extra contract.

## Frontend

- In an existing Tailwind project, prefer utilities over component-local CSS when the result stays readable and uses project tokens. Do not add Tailwind just to follow this preference.
- Use plain CSS for shared tokens, pseudo-elements, complex selectors, structural geometry, keyframes or motion, and rules clearer as one cohesive stylesheet. Do not duplicate declarations across utilities and CSS.
- Prefer semantic tokens, project spacing/layout primitives, and the existing dark-mode strategy over one-off values.
- Prefer `rem` for authored lengths, especially typography, spacing, and component sizing. Use `px` when fixed CSS-pixel geometry is intentional or the design system requires it.
- Prefer the project's defined color palette. Add a new color only when no token meets a concrete semantic or contrast requirement; state the reason.
- For greenfield UI, deliberately choose typography hierarchy, spacing rhythm, density, color intent, contrast, and interaction states. Avoid generic centered-card layouts unless they fit the product.
- Avoid adding a wrapper frame around a group of controls or content when the group itself is not a distinct object.
- Build examples from the simplest realistic composition; add visible structure only when it communicates a real relationship, state, or elevation.
- Prefer CSS-first motion. Use `transform` and `opacity` when practical, keep common transitions short, and respect reduced-motion behavior.

## Rust

- Start application internals with concrete types and local functions/inherent
  methods. Use standard-trait generics at real capability boundaries. Do not add
  an interface per implementation, dependency-injection container or speculative
  framework. Runtime heterogeneity and useful type erasure justify `dyn`.
- Prefer owned retained application models and borrowed processing views. Keep
  intentionally borrowed parsers/large shared inputs borrowed or shared when
  their lifetime contract warrants it; do not add an allocation to obey a slogan.
- Start dynamic/persisted workflows with enums and checked transitions. Use
  typestate when a meaningful compile-time protocol guarantee earns extra types.
  Use methods for type-owned invariants and pure functions for transformations.
- Start with one crate and private implementation modules. Add packages, builders,
  feature flags or facades only for current construction/consumer boundaries.
  Curated library re-exports are valid; avoid automatic prelude/re-export layers.
- Use `Rc::clone(&value)`/`Arc::clone(&value)` where making shared-handle duplication
  visible helps review. Do not require a comment on every clone or remove needed
  snapshots merely to reduce clone counts.

- Reuse an adequate standard/project error. Otherwise prefer `thiserror` for
  nontrivial typed errors and `anyhow` with lazy operation context for application
  orchestration that reports rather than branches on failures. Keep recovery
  categories typed even in applications. Do not preinstall either crate, add both
  to a tiny program unnecessarily, or replace a sound existing convention.
- Start synchronous. When new async I/O concurrency is justified and the project
  has no runtime contract, prefer Tokio. Do not introduce a second runtime or use
  async as a CPU parallelism mechanism.
- Prefer directly awaited work before spawning. For open-ended work prefer
  bounded queues and admission before task creation; choose task tracking from
  required results, error observation and graceful-versus-forced shutdown.
  No actor/supervisor framework is required for a small bounded operation.

Use Cargo, stable rustfmt defaults, default Clippy plus justified individual lints,
the built-in test harness and rustdoc absent contrary project requirements. Do
not enable entire pedantic/restriction groups, add a workspace or test runner, or
choose coverage thresholds by habit. Track `Cargo.lock` for reproducible repository
builds and use `--locked` where reproducibility is promised; a published library's
lockfile does not pin its consumers, so check supported dependency ranges too.
Select dependency-policy checks, fuzzing, Loom and Miri for the actual risk rather
than adding all tools automatically.
