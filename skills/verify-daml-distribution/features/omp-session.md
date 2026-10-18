# Isolated installed OMP session

A user registers the installed native `daml-lsp` in an LSP client and receives
current syntax diagnostics and canonical formatting. This recipe drives the
installed OMP executable, not an OMP source checkout.

From the verification skill directory, run `python3 scripts/archive.py ARCHIVE
--omp /absolute/omp --evidence /absolute/new/directory` as one command. The helper
verifies/extracts the archive, moves its installation, creates an isolated project
and client registration, and loads the verification-only probe explicitly.

Observable success: UTF-16 capability; syntax error at line 1, column 7 after
supplementary Unicode; diagnostics clear after replacement; full-document edit
covers the correct original range; applying it makes formatting idempotent;
installed `daml-fmt --check` accepts the file; OMP exits zero without a model turn.
The retained summary identifies archive/build and installed OMP version.

Host HOME/config/data/cache and credentials are not inherited. Shared LSP state
is disabled. The placeholder startup key is not a usable credential, and no
prompt is sent. This is an RPC/built-in-tool path, not TUI keyboard or GUI proof.
The driver is currently Unix-specific. Normal shutdown and temporary-directory
cleanup remove owned state; retained evidence must remain outside that tree.
