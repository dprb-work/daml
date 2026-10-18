//! Stdio process adapter for the optional DAML language server.

use lsp_server::Connection;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
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
    match threads.join() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("daml-lsp: stdio transport failed");
            ExitCode::FAILURE
        }
    }
}
