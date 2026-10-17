# 2026-09-29 — refactor plan

Kade supplied the lakehouse-k8s CLAUDE.md / MEMORY.md / structure notes and asked for
the useful parts to be carried over, then an all-phases refactor.

**Carried over:** plan-first, a gate per part of the repo, one command per gate,
memory index + `.memory/{active-issues,roadmap,sessions}`, one task one commit
auto-committed after the gate, push stays manual, never report an unrun gate as passed,
2-attempt self-correction cap.

**Left behind (no analogue here):** Helm/k8s context and namespace guards,
`charts/` vs `releases/`, notebooks, environment switching, schema policy, evidence banking.

**Key finding:** `cargo check -p resonance-stream` fails on Linux (`gdk-3.0` missing,
and WinDivert/`windows-sys` are Windows-only), so no Rust test ran in a cloud session.
7 test modules exist, mostly in pure code (`protocol/`, `translator/processor.rs`).
Hence phase 2: move that code into a crate that builds anywhere.

**Follow-up decisions (Kade, after CI went green):** `AppConfig` stays split between app
and ui — architecture, recorded in CLAUDE.md; `pcap`/`windivert` removed;
TROUBLE_SHOOTING deleted (new ones later); footer shows the app version.
