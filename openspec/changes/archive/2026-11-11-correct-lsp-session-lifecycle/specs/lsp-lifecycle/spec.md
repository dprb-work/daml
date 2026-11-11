## Purpose

Keep editor requests, server shutdown and replacement startup within one owned session lifecycle, without stale work, hidden failure or an indefinite business-operation barrier.

## ADDED Requirements

### Requirement: Client retirement closes session admission

A trusted client SHALL close application-message admission for a retiring session before submitting its shutdown request. Every outgoing path SHALL obey the same cutoff, including already-suspended operations, editor providers, context commands, document synchronization, dependency events, trace updates and cancellation traffic. Accepted application writes SHALL remain ordered before shutdown. After shutdown, the client SHALL transmit only the terminal `exit` notification for that session.

A refused request or operation SHALL settle with an explicit local inactive-session or cancellation outcome. Retirement SHALL NOT report successful language work, synthesize remote success, emit an unhandled rejection, or retry a refused operation on a replacement session. Responses to previously submitted requests MAY settle while the old connection remains open; terminal disposal SHALL settle remaining callers without reopening admission.

#### Scenario: A provider resumes across retirement

- **WHEN** a semantic-token request suspends before its wire submission and the session retires before it resumes
- **THEN** the request settles locally without reaching the old server or a replacement server
- **AND** the captured client stream contains no application message after shutdown and no unhandled rejection occurs.

#### Scenario: Watch and cancellation callbacks arrive during retirement

- **WHEN** a queued dependency event, document synchronization, trace update or cancellation callback resumes after its session retires
- **THEN** it cannot publish application traffic on that connection or recreate its watcher ownership
- **AND** any pending caller settles without implying that a remote operation succeeded.

### Requirement: Lifecycle progress does not await business-response completion

Client lifecycle serialization SHALL sequence activation, configuration replacement and disposal, not hold the transition owner for a context or provider response. Session context operations SHALL retain their existing ordering and generation guarantees. A queued operation SHALL remain bound to the session and document identity under which it was admitted; retirement SHALL invalidate queued and unresolved operations for that identity.

#### Scenario: A context response never arrives

- **WHEN** a context operation is pending and configuration replacement or deactivation is requested
- **THEN** retirement can close admission and proceed to its bounded shutdown policy without awaiting that context response
- **AND** the old operation cannot later modify the replacement session or install stale watchers.

### Requirement: Transition outcomes retain ownership

Starting, active and retiring client resources SHALL have one accountable transition owner until they reach a known terminal outcome. Changes to the desired executable during an asynchronous transition SHALL NOT authorize obsolete-session work or repeated intermediate restarts. Deactivation SHALL prevent later activation regardless of queued configuration events. Startup and retirement failures SHALL remain explicit; clearing a client reference or catching a queue rejection SHALL NOT establish successful cleanup or authorize replacement startup.

The client SHALL preserve machine-scoped executable selection, workspace trust admission, cross-platform process support and bounded cleanup of configured non-exec wrappers. Only one session SHALL admit application work at a time; this does not require that every inert retiring wrapper exit before a replacement process can exist. An unknown cleanup outcome SHALL NOT be silently treated as a successful retirement. A missing initialization or shutdown response SHALL have a bounded outcome under the client-owned process policy.

Startup SHALL have a 30-second deadline covering transport acquisition and initialization. Expiry, configuration replacement or deactivation SHALL retire that startup attempt without waiting for initialization and without sending shutdown to an uninitialized server. Late acquisition and initialization completions SHALL remain owned by the retired attempt and SHALL NOT publish an active client or escape cleanup.

Unexpected-close restart decisions SHALL preserve the existing bounded library policy, including its counter history within one configuration lineage. Only the client lifecycle owner SHALL perform a permitted restart. Intentional retirement SHALL NOT consume crash-restart permission or trigger a replacement through a separate library path.

#### Scenario: Initialization never responds

- **WHEN** initialization does not complete before the startup deadline, or deactivation retires it sooner
- **THEN** startup and pending callers settle with the actual timeout or cancellation outcome
- **AND** the captured attempt closes its transport and retains process-cleanup ownership without sending an invalid pre-initialization shutdown.

#### Scenario: A server closes unexpectedly

- **WHEN** the existing bounded restart policy permits replacement after an unexpected close
- **THEN** the lifecycle owner checks the originating session, desired configuration, trust and disposal state before starting it
- **AND** the library cannot bypass that owner or reset its retry history by constructing a replacement client.

#### Scenario: Configuration changes while startup is pending

- **WHEN** the configured executable changes again before a pending startup finishes
- **THEN** the obsolete session is retired without admitting user work
- **AND** only the current desired configuration can become active.

#### Scenario: Disposal fails

- **WHEN** retirement fails or the old session's cleanup outcome is unknown
- **THEN** the failure and its owned resources remain represented
- **AND** a queued context command or configuration event cannot mistake an absent active-client reference for permission to start another active session.

#### Scenario: Deactivation races configuration replacement

- **WHEN** deactivation begins during startup or replacement
- **THEN** no queued event can reactivate the client
- **AND** pending operations settle and session-owned watchers and processes follow the terminal cleanup policy.

### Requirement: Server terminal admission follows the protocol

The server SHALL accept shutdown at most once, return the null shutdown response, and permanently stop ordinary session dispatch. Requests received after shutdown, including repeated shutdown or initialization requests, SHALL receive correlated `InvalidRequest` responses without method-specific parsing or language work. Non-exit notifications and unsolicited responses SHALL NOT change session state or emit language/dependency publications after shutdown. Only an `exit` notification following accepted shutdown SHALL complete normal protocol termination.

The existing synchronous request order SHALL remain intact: work received before shutdown completes before shutdown is accepted. This change SHALL NOT introduce concurrent language execution or a new cancellation-work scheduler.

#### Scenario: A client violates shutdown ordering

- **WHEN** shutdown is followed by a semantic-token, formatting, context or unknown request
- **THEN** each request receives `InvalidRequest` and no ordinary session work executes
- **AND** a subsequent `exit` can still complete normal shutdown.

#### Scenario: A notification follows shutdown

- **WHEN** a document, watched-file or cancellation notification arrives after shutdown
- **THEN** it receives no response and cannot publish diagnostics, dependency updates or language results
- **AND** it cannot reopen session admission.

### Requirement: Terminal waits preserve bounded failure and stdio completion

The server SHALL use one absolute shutdown deadline of 30 seconds from accepting shutdown. Intervening traffic SHALL NOT reset or evade that deadline. Protocol-channel receives and sends SHALL share that budget. Missing `exit`, a broken transport, a failed terminal response or premature `exit` SHALL remain failures with payload-safe error messages.

Normal `shutdown`/`exit` SHALL complete with successful stdio transport termination even if the client keeps stdin open. Failure paths SHALL NOT unconditionally join a stdin reader blocked on that open pipe. This protocol deadline does not promise interruption of an arbitrary blocked OS stdout write; process termination remains the client process owner's responsibility.

#### Scenario: Traffic continues without exit

- **WHEN** a client continues sending traffic after shutdown but never sends `exit`
- **THEN** the single shutdown deadline still expires and the server fails rather than waiting indefinitely or claiming a clean shutdown.

#### Scenario: Stdin remains open after normal exit

- **WHEN** a client completes shutdown and exit while retaining its stdin pipe
- **THEN** the server completes normal transport termination without requiring stdin EOF.

### Requirement: Verification distinguishes prevention from tolerance

Client-side verification SHALL establish wire ordering independently of the server's defensive post-shutdown behavior. Protocol verification SHALL separately exercise a violating peer. Real installed-extension verification SHALL use the corrected client and server together and prove replacement diagnostics after invalid formatting, as well as lifecycle cleanup.

#### Scenario: A tolerant server hides a client defect

- **WHEN** replacement startup succeeds but the captured client still sends application work after shutdown
- **THEN** client lifecycle verification fails even if the server rejects that work and exits cleanly.
