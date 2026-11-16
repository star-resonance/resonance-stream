# 🛠️ Resonance Stream Build Guide

How to build **Resonance Stream** from source. The app is a Cargo workspace:
a Tauri 2 backend (`src-tauri/`), a Leptos frontend compiled to WebAssembly (`src/`),
and two platform-independent crates (`crates/core`, `crates/types`).

There is no Python sidecar and no driver SDK to install. Packets are read from a raw
socket, and the translation engine (llama.cpp `llama-server`, Vulkan build) and the AI
model are downloaded by the app at runtime.

## 📋 1. Prerequisites (Windows 10/11 x64)

* **Rust** (stable, MSVC toolchain) with the WebAssembly target:
  ```cmd
  rustup target add wasm32-unknown-unknown
  ```
* **Trunk** (frontend bundler) and the **Tauri CLI**:
  ```cmd
  cargo install trunk
  cargo install tauri-cli --version "^2"
  ```
* **Node.js**: Trunk's pre-build hook runs the Tailwind CSS CLI through `npx`.
  Install the Node dependencies once:
  ```cmd
  npm install
  ```
* **just** (required for the release build and the check commands): `cargo install just` or `pip install rust-just`.

## 🚀 2. Run in development

The packet sniffer needs **Administrator** rights. Development builds do not embed the
manifest that requests them, so start your terminal as Administrator, then:

```cmd
cargo tauri dev
```

This runs `trunk serve` on port 1420 and starts the app against it.

## 📦 3. Release build

```cmd
package.bat
```

`package.bat` first builds the in-game DLL (`just ingame-build`: it links the C runtime
statically, prints the DLL's imports and fails if it imports a C-runtime DLL). It then runs
`cargo tauri build` (which runs `trunk build` first) with the
`RESONANCE_INGAME_DLL` and `RESONANCE_INGAME_REQUIRED` variables (they only pin the DLL's hash into
the app), and moves the NSIS installer (`*-setup.exe`) into `dist\`. The installer does NOT hold the
DLL: the app downloads its own version's DLL into APPDATA when the in-game send is turned on. A
locally built installer pins a DLL no release has, so its in-game send cannot download one: test
the send with a Candidate build. The DLL is built only this way. Release builds embed `src-tauri/app.manifest`,
so the installed app asks for Administrator rights on launch.

The version number is set once, in the root `Cargo.toml` (`[workspace.package]`).

## ✅ 4. Checks

```cmd
just check
```

Runs formatting, the tests of the platform-independent crates, the frontend check and,
on Windows, the backend check and tests. The same checks run in GitHub Actions on every
push (`.github/workflows/ci.yml`). See `CLAUDE.md` for which check covers which folder.

## ⚠️ 5. Runtime notes

- **First launch downloads:** the AI model (its URL comes from the project's metadata
  gist), the `llama-server` Vulkan build (from this repository's releases) and the user
  dictionary. They are stored in `%APPDATA%\com.enjay.bpsr.resonance-stream`.
- **Firewall:** on first launch the app asks to add a firewall rule for packet capture.
- **Administrator rights** are required for the raw socket.
