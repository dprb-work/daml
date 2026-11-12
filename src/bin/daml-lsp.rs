//! Stdio process adapter for the optional DAML language server.

use lsp_server::Connection;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--version")) {
        if args.next().is_none() {
            println!("daml-lsp {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        eprintln!("usage: daml-lsp [--version]");
        return ExitCode::from(2);
    }
    if let Some(arg) = args.next() {
        if arg == "--help" && args.next().is_none() {
            println!(
                "usage: daml-lsp\nServe the Language Server Protocol over standard input/output."
            );
            return ExitCode::SUCCESS;
        }
        eprintln!("usage: daml-lsp");
        return ExitCode::from(2);
    }
    let (connection, threads) = Connection::stdio();
    if let Err(error) = daml::lsp::serve(connection) {
        eprintln!("daml-lsp: {error}");
        // The executable owns process termination. A failed handshake may leave
        // the client's stdin pipe open; joining that blocked reader would hang.
        return ExitCode::FAILURE;
    }
    // The retained lsp-server reader stops after forwarding exit, without waiting
    // for stdin EOF. Only a completed handshake reaches this transport join.
    match threads.join() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("daml-lsp: stdio transport failed");
            ExitCode::FAILURE
        }
    }
}
