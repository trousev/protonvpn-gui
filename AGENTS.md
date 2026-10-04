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
| `crates/protonvpn-gui/` | UI only: the window, the console pane, the tray, autostart |
| `crates/protonvpn-core/src/engine.rs` | the one thread that owns state; the only writer of the log bus, the interpreter state and the lease |
| `packaging/` | AppImage build script, `.desktop`, icon |

## Commands

```sh
cargo test                                  # unit tests + fixture-corpus guarantees
cargo fmt --all                             # formatting is enforced
cargo clippy --all-targets -- -D warnings   # warnings are errors
./scripts/check-linux-deps.sh               # Linux-only graph, and a ratcheted crate count
./scripts/capture-fixtures.sh               # re-capture fixtures, disconnected set (safe)
./scripts/capture-fixtures.sh --connected   # also brings the VPN up and back down
./packaging/appimage/build.sh               # AppImage
./scripts/release.sh --dry-run              # build and package a release without publishing
./scripts/release.sh --print-version        # the tag the next merge would publish
```

CI runs exactly these four gates — fmt, dependencies, clippy, tests — and `main` cannot move until
they are green.

The dependency gate exists because the graph is the one thing that grows without anyone deciding
to grow it. This application is Linux only: it is not built, tested or shipped for Android, Windows
or macOS, and `scripts/check-linux-deps.sh` fails if a build would compile a crate belonging to one
of them. `Cargo.lock` will still *list* such crates, because it is a union over every target and a
dependency cannot be told to drop its `[target.'cfg(windows)'.dependencies]` table; that is not the
thing worth policing. The count is ratcheted at the value in the script, so adding a dependency
means editing a number and saying why in the commit message — the same way the dependency count
reached two hundred unnoticed otherwise.

## Branch policy

`main` is protected, for the maintainer as much as for anyone else: no pushes, no force pushes, no
deletions, and no merges without a green `test` check. Every change goes through a pull request. No
review is required, but CI is.

Releases are published by `.github/workflows/release.yml` on every merge to `main`
(`scripts/release.sh`), version `X.Y.N`: `X.Y` is the latest release tag, `N` is the commit count.
Nothing is bumped by hand.

Running the GUI without a display, for a smoke test: `sway` with `WLR_BACKENDS=headless` plus
`Xwayland`, then `ffmpeg -f x11grab` to photograph the window. The engine's tests never touch the
real CLI — they drive a stand-in script through `EngineOptions::program`.

Workflows are worth running before they run for real:

```sh
# a local runner image that has rustup and gh, which the slim act images do not
docker build -t pvpn-act:24.04 - <<'EOF'
FROM catthehacker/ubuntu:act-24.04
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates build-essential gh \
 && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.97.1 --component rustfmt --component clippy --no-modify-path
ENV PATH=/root/.cargo/bin:$PATH
EOF

act pull_request -W .github/workflows/ci.yml -P ubuntu-24.04=pvpn-act:24.04 --pull=false
act push         -W .github/workflows/release.yml -P ubuntu-24.04=pvpn-act:24.04 --pull=false -s GITHUB_TOKEN=
```

With an empty `GITHUB_TOKEN` the release script stops after packaging, so `act` exercises
everything except the upload. The provenance attestation needs GitHub's OIDC endpoint and can only
be checked by a real run.

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
- **winit cannot hide a window on Wayland** — `set_visible` is literally "Not possible on
  Wayland". "Close to tray" is therefore destroy-and-recreate, and the app must be an
  `iced::daemon`: an `iced::application` exits the moment its last window is destroyed, which
  would make closing the window quit the app.

## Current state

**Done:** research; architecture; workspace; the PTY driver (`src/pty.rs`); the capture harness
(`src/bin/capture_fixtures.rs`, `scripts/capture-fixtures.sh`); a frozen PTY fixture corpus with
guarantee tests. Then Phase 1 and most of Phase 2: `runner.rs`, `logbus.rs`, `interpreter.rs`,
`parse.rs`, `launcher.rs`, `poll.rs`, `probe.rs`, `net/natpmp.rs`, `qbittorrent.rs`, `config.rs`
and `engine.rs`, plus the `iced` window, the console pane, the `ksni` tray and autostart. Then the
project's own plumbing: protected `main`, CI on every pull request, and a release per merge. Then
the connection manager and the redesigned window ([`docs/architecture.md`](docs/architecture.md)
§11): country, city, P2P, Secure Core, Tor and port forwarding are properties of a **saved
connection**, the shell is a light two-page window (Обзор / Настройки) with the console pinned
underneath, and the login form is a page rather than a tab. `design/after/` holds screenshots of
the result. Then a dependency audit: iced's unused `auto-detect-theme` — and behind it
`dark-light`, a second `zbus` stack, `dconf` and a desktop-sniffing crate — is gone, taking the
Linux closure from 241 crates to 218 and `Cargo.lock` from 384 entries to 321, and
[`scripts/check-linux-deps.sh`](scripts/check-linux-deps.sh) now holds the line. Then the release
pipeline: every merge publishes an AppImage alongside the tarball, assembled in a read-only job
from a toolchain pinned by version and SHA-256, so the third-party `linuxdeploy`/`appimagetool`
never runs in the job that can write.

**Next:** a live `signin` run with real credentials (needs a human — the password prompt is
captured, the 2FA prompt is not); a live port-forwarding check against a P2P server; desktop
notifications, which need a human decision because they would be a fourth sanctioned exception.

**Verified live** (2026-09-30, two full rounds): the app connects on start to the configured
country and the tray reports it, `protonvpn status` agrees, the egress address changes and comes
back, disconnecting from the tray menu takes effect within seconds, quitting from the tray leaves
no window and no bus name, and no NetworkManager profile or egress change is left behind.
