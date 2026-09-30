# AGENTS.md

Guidance for AI agents working in this repository. Read this file first, then
[`docs/architecture.md`](docs/architecture.md) — that is the design contract, and where it
disagrees with anything else, including [`docs/plan.md`](docs/plan.md), it wins.

## What this is

A console-first GUI for Proton VPN on Linux. It is a **wrapper** around the official `protonvpn`
CLI and does not pretend otherwise: every command it runs, and every byte of output, is visible
in a collapsible console pane. That pane is the core of the product, not a debugging
afterthought — the two systems on top of it are an interpreter that reduces the console stream
into state, and a launcher that maps intents to argv and never interprets results.

## The one rule that matters most

> **The only program this application executes is `protonvpn`.**

We do not know, and must not learn, how the connection works. NetworkManager, D-Bus, the
keyring, `/run/user/$UID/Proton/VPN/forwarded_port`, and Proton's own `settings.json` and
`app-config.json` are all **forbidden**. If the CLI says it is connected, it is connected. State
comes from `protonvpn status`, never from inspecting the system.

The temptation to "just read NetworkManager, it's easier" will be strong, and it is the single
most likely way to destroy this design. Do not.

Exactly three exceptions are sanctioned and bounded —
[`docs/architecture.md`](docs/architecture.md) §0 has the table: a `curl` ground-truth probe,
NAT-PMP for the port-forwarding lease, and an opt-in qBittorrent integration on localhost. Do not
add a fourth without a human decision.

## Where things are

| Path | What |
|---|---|
| `docs/architecture.md` | **the contract** — layers, console, interpreter, launcher, polling, exceptions, decisions |
| `docs/cli-surface.md` | captured behaviour of `protonvpn` 1.0.3 — read before writing any parser |
| `docs/plan.md` | roadmap, phases, risk register |
| `docs/research.md` | how the official Proton stack works. Background; not a design input |
| `crates/protonvpn-core/` | all VPN logic. **No UI dependency** — the tray must work with no window |
| `crates/protonvpn-core/tests/fixtures/pty/` | the frozen parser corpus (13 invocations + metadata) |
| `crates/protonvpn-gui/` | UI only — **not created yet** |

## Commands

```sh
cargo test                                  # unit tests + fixture-corpus guarantees
cargo fmt --all                             # formatting is enforced
cargo clippy --all-targets -- -D warnings   # warnings are errors
./scripts/capture-fixtures.sh               # re-capture fixtures, disconnected set (safe)
./scripts/capture-fixtures.sh --connected   # also brings the VPN up and back down
```

Both `cargo fmt --check` and `clippy -D warnings` must be clean before committing.

## Conventions

- Rust 2024; `rustfmt` defaults; clippy clean under `-D warnings`.
- Commit messages: conventional-commit prefix, then prose explaining **why**. `git log` is the
  template — the existing messages are deliberately detailed because the reasoning is the part
  that does not survive in the code.
- `protonvpn-core` must never depend on a GUI toolkit.
- Prefer making illegal states unrepresentable over checking for them at runtime.
- Comments explain reasoning and measured facts. Not what the next line does.

## Things that will bite you

- **Write parsers against `tests/fixtures/pty/`, not against what you remember.** Tests pin the
  corpus properties (no ANSI, width-independent tables, CRLF endings). If one fails, that is a
  signal about the CLI — not a test to relax.
- **The CLI exits after connecting.** The tunnel survives because NetworkManager owns it; the
  in-process Local Agent does not. Anything that must outlive the command has to be ours.
- **`protonvpn status` costs about one second** — every call spawns a Python interpreter. Never
  poll on a short fixed timer. Idle cadence is ≥5 minutes, with extra fresh reads when the user
  looks (window open, tray click).
- **Geolocation from the probe is not evidence.** `ifconfig.co` and `ipinfo.io` report *different
  countries for the same IP* — see `docs/cli-surface.md` §4.9. Only "did the egress IP change
  from the pre-connection baseline" means anything.
- **The CLI refuses to run while the official GTK app is running**, because it checks for the
  session-bus name `proton.vpn.app.gtk`. Our app must never own that name.
- **`protonvpn signin` prompts on a TTY**, which is why every invocation goes through a PTY
  rather than a pipe. One code path, not two.
- **Capturing fixtures and connecting both change this machine's network.** Ask a human first.
- **`docs/research.md` records two retracted conclusions** — NetShield breaking on client exit,
  and GeoIP consistency checking — kept deliberately, with the evidence that overturned them. Do
  not re-derive the old conclusions from the surrounding text.
- **State must be shown with its age** (`updated 3 mins ago`), never as a bare verdict and never
  with the word "stale". See `docs/architecture.md` §7.

## Current state

**Done:** research; architecture; workspace; the PTY driver (`src/pty.rs`); the capture harness
(`src/bin/capture_fixtures.rs`, `scripts/capture-fixtures.sh`); a frozen PTY fixture corpus with
guarantee tests.

**Next — Phase 1:** `runner.rs` (serialized invocations, streamed lines, full invocation
records), `logbus.rs` (one ordered stream, two consumers: console and interpreter),
`interpreter.rs` (a pure reducer over the fixtures). Then the `iced` + `ksni` GUI — the framework
choice is a default, not a settled decision, and is meant to be confirmed when the UI itself is
discussed. Nothing in the core depends on it.
