## Context

See [proposal.md](proposal.md) for accepted scope and the owner's implementation authorization. Baseline: `a9686511027623c1c1284cd99a8e4ca881809c80`. That baseline's relevant lifecycle code matched the investigated v0.2.1 path.

The original CI reached restart, reported a shutdown-handshake failure, and timed out awaiting replacement diagnostics. A narrow real-host wire probe using the original v0.2.1 Linux executable and VSIX identified a late semantic-token request after shutdown acknowledgment. The original CI did not capture its intervening message; do not claim that its exact message is known.

Three current owners fail to compose:

- `integrations/vscode/src/extension.ts` serializes `startClient`, `stopClient` and `selectContext` through `enqueue`, but `selectContext` retains the queue for a remote response. `stopClient` clears `client` before disposal settles. Queue recovery can therefore mistake failed retirement for an idle client.
- `vscode-languageclient` 9.0.1 checks running state before asynchronous connection acquisition/full-document synchronization. A suspended provider can resume on a retiring connection. Default crash restart also calls library startup outside the extension queue.
- `src/lsp/mod.rs` delegates shutdown to `lsp-server` 0.10.0. That convenience helper acknowledges shutdown and requires `exit` as the next message, rather than rejecting late requests while awaiting exit.

There is a fourth, independently demonstrated settlement defect. JSON-RPC 8.2.0 registers request tracking after awaiting the writer and rethrows inside an ignored asynchronous Promise executor after rejecting a failed write. A standalone probe with a rejecting writer produced both a caller rejection (`-32099`) and `unhandledRejection`. Disposal during an outstanding write can also miss the not-yet-registered request.

## Goals / Non-Goals

**Goals:** one owner per transition, one final wire-admission boundary per connection, explicit outcomes for pending operations, and protocol-correct server termination. Correct ordering must be proven separately from defensive server behavior.

**Non-Goals:** replacing editor providers or the process launcher; introducing concurrent server execution, a general actor framework, retry infrastructure or a new service; changing language behavior, trust, settings, advertised VS Code compatibility, or published release bytes. Exclusive active protocol admission is required, not exclusive existence of OS processes.

## Decisions

### 1. A session record owns the whole transition

Replace the implicit meaning of `client === undefined` with one authoritative record for the starting, active, retiring or failed session. The record binds the client, connection admission, dependency session, desired configuration identity and terminal outcome. Keep this owner local to the extension; it is not a public lifecycle framework.

Configuration events update the desired executable and synchronously revoke obsolete session admission. One reconciliation path performs transitions. While a transition awaits, later configuration intentions replace obsolete intentions rather than enqueue redundant intermediate restarts. A startup completion cannot publish an active session unless its configuration is still desired, trust still holds, and deactivation has not been requested.

Retain the retiring record until its result is known. A stop rejection remains a failure even if the library clears its connection field or sets its public state to stopped. A context command never authorizes recovery from a failed retirement. Deactivation is terminal for admission; later callbacks cannot reactivate the extension.

Only lifecycle transitions occupy this queue. Context commands acquire a session identity, then complete outside it. Preserve per-session context sequencing and generation checks in `DependencySession`; do not replace serialization with blanket concurrent context selection. Retirement invalidates that session's queued and unresolved context operations, watchers and callbacks. An operation never migrates to a new session.

The reconciliation request itself owns a pending signal and its completion promise. Requests arriving during a transition join that owner and mark another pass. The final pending check and release of shared ownership occur in the same worker step, without a chained `finally` microtask gap. Preserve the first failure while draining explicit requests; an error does not create an automatic retry.

Keep the existing bounded crash-restart policy, but route its effects through this owner. The public `createDefaultErrorHandler` provides the stock decision and counter state. Delegate to that retained handler while returning `DoNotRestart` to the library; after the supported `handleConnectionClosed` override finishes stock cleanup, submit its recorded restart decision to reconciliation. Preserve the handler across crash replacements in one configuration lineage, resetting only for an explicit new configuration as today. Delegate ordinary error decisions too. Intentional retirement does not become a crash retry, and logs must not claim a restart that the owner refused.

### 2. Fence the actual outgoing writer, once per connection

Use the supported protected `LanguageClient.createMessageTransports` seam. Call the stock implementation and wrap its writer, retaining its reader and its executable/process support. Do not replace executable `ServerOptions` with a custom launcher.

This is the one final admission owner. General middleware cannot cover direct full-document flushes, progress, trace notifications, JSON-RPC cancellation or replies to server requests. Do not add parallel method-specific guards and claim that their collection is a proof.

The gate checks synchronously before forwarding a write. Already-admitted writes retain the stock FIFO order. Closing ordinary admission before submitting shutdown prevents an already-suspended provider from crossing the boundary when it resumes. After the cutoff, only the lifecycle's shutdown and acknowledged-shutdown `exit` traffic are allowed. Late requests, notifications and delayed replies settle locally; they never become synthetic remote success or get replayed on the replacement.

Carry that outcome through the registered SDK provider too. The pinned `handleFailedRequest` otherwise converts inactive and disposed-request errors into a successful `null` default. The session adapter preserves explicit inactivity, translating closed/disposed connection outcomes with their original cause; unrelated errors retain SDK handling.

Bootstrap is also owned. Permit the required initialization exchange and enable ordinary messages at successful `initialized` submission only if the session remains current. The gate must open before feature initialization emits document synchronization, not merely after `start()` resolves. A late bootstrap continuation cannot reopen a retired connection.

Do not wait for all remote request responses before shutdown. That would turn one missing response into an indefinite retirement barrier. Previously submitted requests may settle while the old reader is active; final connection disposal rejects the rest.

### 3. Correct dependency settlement before relying on local rejection

A writer-only fence is inadmissible on the current graph: its rejection exposes the confirmed unhandled-rejection defect. Correct the pinned JSON-RPC owner to register requests before asynchronous writing and remove them on write failure, reject exactly once, clean cancellation listeners, and avoid ignored executor failures. Give deliberate retirement a local inactive/cancelled classification, not a generic unexplained transport failure.

The language-client stop owner must dispose its captured connection on failure and timeout as well as successful shutdown. Clearing `_connection` in a `finally` block does not settle pending callers. A bounded cancellation of startup must likewise close its captured connection, settle startup, and retain the existing Node process owner's cleanup; sending shutdown before initialization is not a legal substitute.

Terminal disposal also finalizes leftover SDK-owned diagnostic channels when connection-close handling has already set Stopped and cleared connection state. Caller-owned channels remain caller-owned. Deferred raw-process diagnostics use their captured attempt's ownership; retired callbacks keep pipes drained but cannot acquire a new channel or publish into a replacement.

Extend the existing `stop`/`dispose` path to handle `Starting`; do not add another manager or pretend stock `stop` already supports it. One captured startup-attempt identity owns its cancellation latch, connection/transports and start/stop settlement. Retiring that attempt cancels startup, closes acquired resources and never sends shutdown or exit to an uninitialized server. Every asynchronous acquisition/initialization continuation checks that same identity before publishing a connection, running feature initialization or setting Running. Retired continuations dispose late resources instead of overwriting a newer attempt.

Propagate startup cancellation through the existing protected transport-acquisition path to the Node owner. Check cancellation immediately before spawn after asynchronous working-directory resolution, and keep cleanup ownership of a child already spawned. A late acquisition must not escape the old attempt's cleanup or clear another attempt's process handle. Observe cancellation and bootstrap-write failures through the same settled start result.

Propose a 30-second startup deadline from beginning the attempt, including transport acquisition and initialization. Expiry requests retirement of that exact attempt directly, not behind the queue awaiting its start promise. This is a new explicit bound replacing an indefinite wait; slow configured wrappers are a compatibility risk for the owner to accept. Process cleanup retains its existing separate bounded policy, not a promise of hard real-time OS termination. Configuration change and deactivation can retire startup before the deadline. Whichever original error or intentional cancellation wins remains the reported outcome; later callbacks cannot overwrite it.

Prefer a released compatible dependency graph only when its packaged implementation establishes these contracts. Current stable client 11.0.0 advertises VS Code `^1.91.0`, while DAML advertises `^1.85.0`; it is not a drop-in adoption under this scope. An upstream source fix is evidence for a backport, not proof that an installed release meets every lifecycle obligation.

The bounded adoption route is versioned corrections to the pinned packages. One explicit preparation command applies them after `npm ci --ignore-scripts`, checks exact versions and source identity, and fails on drift. Build, typecheck and packaging use that same owner; do not depend on `postinstall`, hand-edit `node_modules`, publish a fork or introduce runtime fallback patching. Record provenance, behavioral checks and removal conditions. Preserve the packaged `terminateProcess.sh` asset and existing license/notice generation.

There are two separate corrections, not two admission gates: JSON-RPC owns promise settlement; the language client owns terminal connection/start cleanup. The final writer alone owns outgoing admission.

### 4. Normal protocol policy stays at the server dispatch boundary

Keep `serve` as the single protocol-admission owner. Retain its initialization barriers and synchronous session dispatch. Replace only the incomplete shutdown helper with a terminal phase:

- Accept shutdown once and close ordinary session dispatch permanently.
- Return its null acknowledgment, releasing retained session state where practical.
- Reject every later request with correlated `InvalidRequest`, including duplicate shutdown and malformed method parameters, without ordinary method parsing.
- Discard non-exit notifications and unsolicited responses; do not publish diagnostics or dependencies or reopen admission.
- Complete normal termination only on `exit`.

Use one absolute 30-second deadline for terminal channel receives and sends, checking expiry between messages. Do not reset the budget for intervening traffic. Ready channel messages can otherwise evade an expired receive deadline. A failed acknowledgment send remains a failure; the current helper discards that result.

No pending-work scheduler is needed on the server: synchronous session work received before shutdown already finishes before shutdown is dispatched. Cancellation must not introduce concurrent work or reopen terminal admission.

### 5. Protocol retirement and process cleanup are distinct

Normal `shutdown`/`exit` lets the dependency stdin reader stop after forwarding `exit`, even if stdin remains open. Preserve successful transport joining and failure reporting. On handshake/transport failure, do not unconditionally join a reader that may be blocked on stdin.

Channel deadlines do not interrupt an arbitrary blocked OS stdout write. Keep process termination with the existing Node client process owner, including bounded non-exec-wrapper cleanup. Do not add executable supervision to claim a stronger server guarantee than the transport supports. A retiring wrapper may exist briefly while another protocol session starts, but it cannot regain active admission and its cleanup ownership cannot be discarded.

## Alternatives considered

| Alternative | Decisive consequence |
| --- | --- |
| Server tolerance only | Repairs protocol behavior but leaves late client traffic, blocked lifecycle progress and lost retirement ownership. |
| Provider or middleware guards only | Misses direct synchronization, cancellation, trace, progress and RPC replies. |
| Writer fence alone | Covers bytes but creates an observed unhandled rejection and can strand requests on the pinned graph. |
| Drain every response before stopping | A nonresponding business operation prevents shutdown. |
| Replace the client/framework or own a new process manager | Assumes provider, synchronization, transport and platform responsibilities that the existing owners already carry. |
| Session ownership plus final admission and bounded dependency corrections | Addresses the actual gaps while preserving feature and process owners; adds explicit patch maintenance. |

## Verification and evidence limits

Freeze checks before production changes. Deterministic barriers must suspend sends at the relevant awaits; timing sleeps and a passing sequential restart are not concurrency evidence. Capture the outgoing stream to fail any post-shutdown ordinary message, even when a compliant server tolerates it.

Check request tracking while writes are pending, rejection without `unhandledRejection`, disposal with unanswered requests, a context response withheld across retirement, obsolete startup, rapid configuration changes, deactivation, failed stop, late watcher/trace/cancellation/reply traffic, and non-exec wrappers. Preserve context generation ordering and trust refusal. Separately exercise protocol violations, absolute deadline expiry under traffic, open-stdin clean exit and broken transport failure. Finish with the installed VSIX/native binary scenario that verifies replacement diagnostics after invalid formatting.

Observe actual registered provider rejection, not only the writer/RPC result. Release a genuine dependency-producing reply across retirement and drain its continuations before checking real watcher ownership. Exercise the configuration completion window with real batched callbacks/snapshots. Private terminal-phase checks use the existing channel implementation as an exact-version dev dependency to force output backpressure and already-ready expired operations, without a public test API or OS-write-preemption claim.

The original wire captures establish the old race and the standalone rejection probe rejects a writer-only design. Implementation evidence belongs to the task completion and exact-revision review; design acceptance alone establishes neither working behavior nor release readiness.

## Risks / Trade-offs

The final transport seam is supported, but patched dependency lifecycle internals become an explicit maintenance responsibility. Bootstrap cancellation and terminal error classification need focused runtime proof before the client change can be accepted. A new upstream release removes a patch only after the same behavioral checks pass, not by version number alone. Public minimum-editor support and process behavior must not change incidentally.

The new failed-retirement fixture exposed an output-channel factory warning in
pinned VS Code 1.105.1. A separate public-API-only probe reproduced the same four
disposed-store warnings without loading DAML: immediate channel append/dispose
races the host's asynchronous registration. The host source
[`extHostOutput.ts`](https://github.com/microsoft/vscode/blob/1.105.1/src/vs/workbench/api/common/extHostOutput.ts)
establishes that separate owner. This does not establish zero host resource
leaks. Host upgrade/repair is outside this change; no warning suppression,
test delay or private host patch is used as a substitute.

## Accepted scope

The owner accepted the integrated scope, versioned dependency corrections and 30-second startup deadline as recorded in [proposal.md](proposal.md). Crash-restart decisions retain the stock policy, but all restart effects move under session ownership. Startup retirement extends existing dependency lifecycle owners rather than introducing a new process manager. Result acceptance, merge and release remain separate decisions.
