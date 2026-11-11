//! Terminal deadlines cover blocked output and immediately ready channels.

use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{SendTimeoutError, bounded};
use lsp_server::{Connection, Message, Notification, Response};
use serde_json::{Value, json};

use super::{await_exit, send_terminal};

#[test]
fn blocked_terminal_send_expires_without_waiting_for_output_to_drain() {
    let (sender, peer_output) = bounded(0);
    let (_peer_input, receiver) = bounded(0);
    let server = Connection { sender, receiver };
    let (done, completion) = mpsc::sync_channel(1);

    thread::scope(|threads| {
        threads.spawn(move || {
            // This exercises the existing private deadline parameter; the
            // public protocol's fixed 30-second policy is covered separately.
            let deadline = Instant::now() + Duration::from_secs(1);
            let result = send_terminal(
                &server,
                Response::new_ok(1.into(), Value::Null).into(),
                deadline,
            );
            let _ = done.send((deadline, Instant::now(), result));
        });
        let observed = completion.recv_timeout(Duration::from_secs(5));
        // Drop the live, undrained receiver BEFORE asserting or joining. A
        // defective blocking send is then released even when the watchdog fires.
        drop(peer_output);
        let (deadline, completed, result) =
            observed.expect("terminal output must settle without a draining receiver");
        assert!(completed >= deadline, "terminal send must not fail early");
        let error = result.expect_err("undrained rendezvous output must time out");
        let cause = std::error::Error::source(&error)
            .and_then(|cause| cause.downcast_ref::<SendTimeoutError<Message>>())
            .expect("terminal send must preserve the channel timeout cause");
        assert!(
            matches!(cause, SendTimeoutError::Timeout(_)),
            "the receiver stayed connected until after the send deadline"
        );
    });
}

#[test]
fn expired_terminal_send_does_not_emit_even_when_output_is_ready() {
    let (server, client) = Connection::memory();
    let deadline = Instant::now();

    send_terminal(
        &server,
        Response::new_ok(1.into(), Value::Null).into(),
        deadline,
    )
    .expect_err("ready output cannot authorize a write after deadline expiry");
    assert!(
        client.receiver.try_recv().is_err(),
        "an expired terminal send must not emit a response"
    );
}

#[test]
fn expired_terminal_receive_does_not_accept_an_already_queued_exit() {
    let (server, client) = Connection::memory();
    client
        .sender
        .send(Notification::new("exit".to_owned(), json!({})).into())
        .unwrap();
    let deadline = Instant::now();

    await_exit(&server, deadline)
        .expect_err("a ready exit cannot turn an expired shutdown into success");
    assert!(
        client.receiver.try_recv().is_err(),
        "an expired terminal receive must not emit a response"
    );
}
