# Gates for resonance-stream. `just check` runs every gate this OS can run.
# See CLAUDE.md for which gate applies to which part of the repo.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default: check

# Every gate the current OS can run (the app crate builds on Windows only).
check: fmt-check core-check ui-check
    {{ if os() == "windows" { just_executable() + " app-check" } else { just_executable() + " app-cross-check" } }}

# Formatting, whole workspace.
fmt-check:
    cargo fmt --all -- --check

fmt:
    cargo fmt --all

# core part: platform-independent logic, the llama-server client + shared
# types. Runs anywhere.
core-check:
    cargo test -p resonance-core -p resonance-frame -p resonance-llama -p resonance-types

# ui part: Leptos frontend, compiled for the browser target; its pure
# modules (e.g. chat_view) are unit-tested on the host.
ui-check:
    cargo check -p resonance-stream-ui --target wasm32-unknown-unknown
    cargo test -p resonance-stream-ui

# Needs a `dist/` folder: tauri::generate_context! checks frontendDist exists.

# app part: Tauri backend tests. Windows only. `cargo test` type-checks the lib,
# the bin and the tests; a separate `cargo check` would build every dependency a
# second time (check and test artifacts are not shared).
app-check:
    cargo test -p resonance-stream

# app part from Linux: compile-only check against the Windows GNU target.
# Catches type/import errors; cannot link, so it does not run the app tests.
# Setup: rustup target add x86_64-pc-windows-gnu; apt install gcc-mingw-w64-x86-64
[unix]
app-cross-check:
    mkdir -p dist && ( [ -f dist/index.html ] || echo '<!doctype html>' > dist/index.html )
    cargo check -p resonance-stream --target x86_64-pc-windows-gnu

# Line coverage of core + llama + types (needs `cargo install cargo-llvm-cov`).
coverage:
    cargo llvm-cov -p resonance-core -p resonance-frame -p resonance-llama -p resonance-types --summary-only
