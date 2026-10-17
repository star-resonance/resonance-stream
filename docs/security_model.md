# Security Model

Where Resonance Stream crosses a trust boundary, what is checked today, and what is not. Written 2026-10-06 on `main` `1478528` plus later docs.
Companion to [`architecture_review.md`](architecture_review.md) (finding IDs W-1.. and recommendations A-6.. refer to it) and [`decisions.md`](decisions.md).

Evidence labels are the same as in the review: **confirmed** (the code was opened and the `file:line` says it), **reported** (a review agent read it; not checked by hand), **estimated**.
Nothing here was attacked or fuzzed against the real app; "mitigated" means a check exists in the code, not that it was exploited and held.

## 1. Why this app is security-sensitive

- It **asks for administrator rights** (`src-tauri/app.manifest:19`, `requireAdministrator`, confirmed). It needs them for a raw socket with `SIO_RCVALL` (the sniffer, port 5003) and for Windows Firewall rules.
- It **downloads and runs executables**: its own updates and a `llama-server.exe`; it also downloads a model file that llama.cpp parses.
- It **reads game traffic** from the network and shows other players' text. It does not hook or modify the game client (no injection).
- It **stores chat** (daily `.jsonl` logs) of other players in the app data folder: private data, not secrets.

So a flaw that lets any of those inputs change what runs, runs as administrator.

## 2. Actors and inputs

| Actor / input | Trust |
|---|---|
| Game server bytes on TCP 5003 (and anyone who can put packets on the path) | untrusted input |
| The update feed `latest.json` on this repo's GitHub releases | untrusted transport, authenticated by signature |
| The metadata gist (model URL / hash / version, custom dictionary) | untrusted transport, **not signed** (the copies of the app that exist today read it; the next app version reads the signed `metadata` branch instead, D-28) |
| The `metadata` branch of this repo (the signed model / dictionary metadata, from `metadata/` on `main`) | untrusted transport, authenticated by signature (written by `metadata.yml` on a pushed tag `metadata-v<N>`; read by the app from M3b on) |
| The AI-server zip on this repo's releases (`v0.2.0` asset) | pinned by a SHA-256 constant in code |
| Another process of the same Windows user | **not trusted**, but today it can write where the app executes from |
| A tester running the bridge | trusted, but only in a `test-env` build |
| The webview content (the app's own UI) | trusted code, but with no CSP |

## 3. Boundaries: what is checked and what is not

### 3.1 Network packets -> decoder (`crates/core/protocol`, `capture`)

| Check | Status |
|---|---|
| A frame is limited to 1 MiB (`MAX_FRAME_LEN`, `protocol/framing.rs:26`; the assembler buffer stays within it, test at `:416`) | confirmed |
| The assembler, parsers, pipeline and capture-line readers survive any bytes or damaged streams (`crates/core/tests/properties.rs`, `proptest`) | confirmed (tests exist; they pass in `just core-check`) |
| Compressed frames: output capped at 1 MiB, but a hostile `0x8003` frame could make `ruzstd` reserve up to 100 MB first (`frame_decoder.rs:22`) | reported; needs an on-path attacker. Gap: A-1.3 |
| Chat text cannot open or close a prompt turn (`sanitize_input` strips turn markers; golden test `chat_text_cannot_inject_turn_markers`) | confirmed |
| A `[P<n>]` typed in chat collides with a real placeholder | confirmed defect W-11 (rare, not a code-execution path) |

### 3.2 App update (`downloader/gist.rs`, `app_updater.rs`, `core/update_signature.rs`)

| Check | Status |
|---|---|
| The feed is read from `releases/latest/download/latest.json` (`gist.rs:17`); body capped at 256 KB (`:20`, `:51`) | confirmed |
| The download URL must be **HTTPS**; plain `http://` only to this machine and only with a test flag (`core/download.rs:11-35`) | confirmed |
| Only the release the last check announced is downloaded (`app_updater.rs:65-70`), and the announcement is filtered for newer versions (`gist.rs:112-120`) | confirmed |
| Size is compared with `Content-Length` and the SHA-256 is checked when one is announced (`DownloadCheck`, `core/download.rs:60-80`) | confirmed |
| **A minisign signature must verify against one of the built-in public keys** (`TRUSTED_UPDATE_KEYS`: a primary and an offline backup); the signed version must equal the announced one, so an old signed exe cannot pass as new (`update_signature.rs`) | confirmed |
| The swap `current -> .old`, `temp -> current` has **no rollback** if the second rename fails | confirmed gap W-1 (A-6.1) |

Result: whoever controls the feed or the release page cannot get code run without the private signing key. The remaining risk is local: availability of the app after a failed swap.

### 3.3 AI server (`downloader/server.rs`, `translator/server_manager.rs`)

| Check | Status |
|---|---|
| The zip comes from this repo's releases over HTTPS and its **SHA-256 is a constant in code** (`server.rs:6-8`, computed 2026-09-29) | confirmed |
| Extraction is **not atomic**; an existing `llama-server.exe` counts as installed (`server.rs:38-40`) | confirmed gap W-3 (A-6.4) |
| At start the app spawns `<app data>/bin/ai-server/llama-server.exe` with `Command::new`. Since S3a the 22 files are checked against pinned SHA-256 values right before the spawn (`resonance_core::server_pins`), and any other `.exe` / `.dll` in the folder is refused; on a mismatch nothing is started and the user is told | W-2 closed against an edit made while the app is not running. **Open:** the gap between the check and the spawn (a process that swaps a file in that window); a locked folder ACL would close it |
| Only the PID this app started is killed (current PID plus the PID file of a crashed run) (`server_manager.rs:271-294`) | confirmed |
| It binds `127.0.0.1` on 8080 or a free port | per CLAUDE.md; not re-checked |

The zip pin protects the download, not the files later on disk. A process of the same user can replace the exe or a DLL and it runs as administrator at the next translator start (estimated, traced not reproduced).

### 3.4 Model and dictionary (`downloader/model.rs`, `gist.rs`)

| Check | Status |
|---|---|
| `download_model` takes URL and SHA-256 **from the signed metadata, verified at every download** (`gist::verified_metadata_fresh`, core `signed_metadata::model_source`), never from the caller; an empty hash or address is refused (N-5, 2026-10-08) | confirmed reading; the real-app proof is the `download-integrity` runbook |
| The download is checked for length and SHA-256 against that hash (`model.rs:123`); an installed model with a matching hash is not downloaded again (`:76-80`) | confirmed |
| The hash and the URL come from the same unsigned source, so the check protects against corruption, **not against a tampered gist** | confirmed reading; gap W-8 (A-6.3). Being closed (D-28): the verifier (`signed_metadata.rs`) and the signing workflow (`metadata.yml`) exist; **the app reads the signed `metadata` branch from M3b on** (a bad or missing signature, a rollback or a dictionary that is not the named file is refused and logged; `gist.rs`), but copies built before that still trust the gist |
| The signing workflow runs only for a pushed tag `metadata-v<N>` (never for a merge: `claude/*` PRs auto-merge) and uses the same secret as `release.yml`. A workflow file in a tag's own commit is the one that runs, so anyone who can push a tag could run a changed copy with the secret: the same exposure as releases; limiting the secret to an Environment that only tags `metadata-v*` / `v*` may use closes it (a repository setting, optional) | confirmed reading; known |
| A model file is parsed by llama.cpp, which is not sandboxed | estimated |
| The dictionary is parsed as typed JSON (`Dictionary::from_json_str`, invalid input refused: `gist.rs:181,233`); its terms are shielded behind placeholders in the prompt and put back in the answer | confirmed |
| The gist and dictionary fetches use `reqwest::Client::new()`: **no timeout**, and the metadata / dictionary bodies have **no size cap** (`gist.rs:37,78,172`) | confirmed gap W-8 (A-6.5) |

### 3.5 Webview and Tauri permissions

| Check | Status |
|---|---|
| `"csp": null`, `"withGlobalTauri": true` (`tauri.conf.json:23,12`) | confirmed gap W-9 (A-6.6) |
| `capabilities/default.json` granted `shell:allow-spawn`, `shell:allow-execute` (a sidecar `bin/translator` that does not exist) and `opener:default` | **removed in S3c-1**: nothing in Rust or the ui used the shell plugin (its init and dependency are gone too) or the opener's JS API; `open_browser` is a Rust command and needs no webview permission |
| `open_browser(url)` handed any string to the opener (`io/fs.rs`) | **closed in S3c-1**: `check_open_url` accepts only a plain https page (host present, no space, control character or backslash) |
| No `inner_html` on chat text was found | reported |

### 3.6 Local state

| Check | Status |
|---|---|
| Config and metadata writes are atomic in some places (`write_atomic`), plain `fs::write` in others (`metadata.rs:60`, `gist.rs:186,234`) | reported; gap W-7 |
| A failed config read replaces the settings with defaults without a backup (`app_config.rs:226`) | confirmed gap W-7 |
| The firewall rule is per exe (the name carries a hash of the exe path), so a dev exe and the installed exe do not delete each other's rule (`core/sniffer_net.rs`) | confirmed |
| Chat logs contain other players' messages in plain text in the app data folder | by design; retention is configurable (`chat_log_retention_days`) |

### 3.7 The test bridge

| Check | Status |
|---|---|
| Compiled only with the `test-env` feature; a stable exe holds neither the bridge nor its MQTT client (release workflow test: "stable release never mentions test-env", `rc-lib.test.sh`) | confirmed |
| Connects only to `mqtt://127.0.0.1`, `localhost` or `[::1]` (`core/test_env.rs:222,372`) | confirmed |
| `--llama-url`, `--metadata-url`, `--feed-url`, `--dictionary-url` accept only `https://` or loopback `http://` (`test_env.rs:226,230`); a downloaded exe must still carry a valid signature even then | confirmed |
| `--metadata-trust-key` makes a test run trust one throwaway key for the signed metadata **instead of** the built-in keys. Like every flag it is read only in a debug or `test-env` build (`GATE_OPEN`), and `release.yml` never names `test-env` (pinned in `rc-lib.test.sh`) | confirmed reading |
| Commands are a **fixed allow-list** (`Command` in `core/bridge.rs`); an unknown name is refused with an error | confirmed |
| An answer over 16 MiB becomes an error ack (`fit_ack`), not a dropped connection | confirmed (PR #203) |

## 4. Threat scenarios

| # | Scenario | Today | Gap |
|---|---|---|---|
| T-1 | A hostile host serves a fake update | rejected: HTTPS, announced release only, minisign with version binding | none known |
| T-2 | A process of the same user replaces `llama-server.exe` or a DLL | runs elevated at the next translator start | **W-2** (checked before spawn since S3a; the check-to-spawn window stays open) |
| T-3 | The metadata gist is changed to point at another model URL and hash | accepted by copies built before M3b: both come from the gist; refused by the app from M3b on (D-28) | **W-8** |
| T-4 | An update swap fails halfway (antivirus, lock) | the app is left without an exe | **W-1** |
| T-5 | The app is killed while extracting the AI server | the half-extracted folder counts as installed | **W-3** |
| T-6 | An on-path attacker sends crafted game frames | decoder is clamped and property-tested; zstd window can reserve memory | A-1.3 |
| T-7 | A player writes control tokens or `[P<n>]` in chat | tokens are stripped; `[P<n>]` collides | W-11 |
| T-8 | A page of the webview is hijacked | no CSP (S3c-2, waits on a Windows test); any URL can be opened and leftover shell permissions (both closed, S3c-1) | **W-9** |
| T-9 | A hung remote host blocks start-up | no timeout on gist / dictionary fetches | W-8 |

## 5. Rules for new code

1. **What runs elevated is verified when it is run**, not only when it was downloaded (hash or signature of the file that is about to be spawned).
2. **Remote data that is not signed is data, not configuration.** Parse it into typed fields, cap its size, set a timeout, and never take an executable's URL **and** its hash from the same unsigned source.
3. **HTTPS only**; a plain-HTTP URL is accepted only for this machine and only behind a test flag (`check_download_url_allowing`).
4. **Atomic file changes**: write to a temporary name and rename; a swap must have a way back.
5. **Never kill or touch a process this app did not start.**
6. **No new shell permission or capability** without a reason written next to it; remove the ones that are unused.
7. **The bridge stays behind `test-env`**: no new command that is not in the fixed list, no non-loopback address.
8. **No secrets in the repo.** The gist, feed and release URLs and the update public keys are public; the private signing keys are not here (CLAUDE.md).

## 6. What to do about the gaps

In order of risk; each item is a stage in the roadmap (`.memory/roadmap/refactor.md`).

| Gap | Fix | Roadmap |
|---|---|---|
| W-1 no rollback in the update swap | `install_swap` in core; rename `.old` back on a second failure | S1a |
| W-3 non-atomic extraction | extract to `.part`, then rename | S1b |
| W-2 unchecked server exe and DLLs | ~~per-file SHA-256 pins verified just before spawn~~ done (S3a); a locked folder ACL remains an option for the window between check and spawn | S3a |
| W-8 unsigned metadata, no timeouts | minisign check of the gist metadata with the built-in keys; timeouts and size caps | S3b |
| W-9 webview | ~~remove unused shell permissions, `open_browser` only `https`~~ done (S3c-1); CSP open (S3c-2) | S3c |
| A-1.3 zstd window | `set_max_window_size` | with S4 or S5 |

## 7. Not verified

- Nothing in sections 3 and 4 was exploited; "mitigated" means a check exists in code.
- Items marked **reported** were read by a review agent and not checked by hand.
- Not checked: whether `config.log_level` reaches the backend logger, the write permissions of the NSIS install folder, and the firewall rule's scope.
- The windows-only code (`src-tauri/`) was read, not run, in this review.
