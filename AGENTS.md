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

Exactly four exceptions are sanctioned and bounded —
[`docs/architecture.md`](docs/architecture.md) §0 has the table: a `curl` ground-truth probe,
NAT-PMP for the port-forwarding lease, a loopback-only SOCKS5 proxy that refuses to relay unless
the route it pinned still holds (§13), and a `curl` to **this project's own release page**
for the AppImage updater, which verifies what it downloads and never executes it (§14). Do not add
a fifth without a human decision.

## Where things are

| Path | What |
|---|---|
| `docs/architecture.md` | **the contract** — layers, console, interpreter, launcher, polling, exceptions, decisions |
| `docs/cli-surface.md` | captured behaviour of `protonvpn` 1.0.3 — read before writing any parser |
| `docs/plan.md` | roadmap, phases, risk register |
| `docs/research.md` | how the official Proton stack works. Background; not a design input |
| `crates/protonvpn-core/` | all VPN logic. **No UI dependency** — the tray must work with no window |
| `crates/protonvpn-core/tests/fixtures/pty/` | the frozen parser corpus (13 invocations + metadata) |
| `crates/protonvpn-gui/` | UI only: the window, the console pane, the tray, the `.desktop` entries |
| `crates/protonvpn-core/src/engine.rs` | the one thread that owns state; the only writer of the log bus, the interpreter state, the lease and the proxy's gate |
| `crates/protonvpn-core/src/socks5.rs` | the local SOCKS5 proxy (exception #3): the protocol, the listener, the counters |
| `crates/protonvpn-core/src/net/route.rs` | the kernel's source-address answer the proxy's gate is built on — a connected UDP socket that is never written to |
| `crates/protonvpn-core/src/update.rs` | the AppImage updater (exception #4): the check, the checksum, the swap |
| `crates/protonvpn-core/i18n/` | the message catalogue — one directory per language, English first |
| `crates/protonvpn-core/build.rs` | compiles the catalogue and **fails the build** on an incomplete translation |
| `docs/i18n.md` | the localization rules, in prose |
| `scripts/translate` | fills in what is missing, with an LLM; run it before opening a pull request |
| `scripts/version.sh` | the one definition of `X.Y.N` — published as a tag and baked into the binary |
| `packaging/` | AppImage build script, `.desktop`, icon |

## Commands

```sh
cargo test                                  # unit tests + fixture-corpus guarantees
cargo test -p protonvpn-core --test live_update -- --ignored --nocapture
                                            # the updater against the real release page (~5 MB);
                                            # ignored on purpose: `cargo test` never reaches the
                                            # network, and CI never sees it
cargo fmt --all                             # formatting is enforced
cargo clippy --all-targets -- -D warnings   # warnings are errors
./scripts/check-linux-deps.sh               # Linux-only graph, and a ratcheted crate count
./scripts/translate                         # translate what is missing, then verify the build
./scripts/translate --dry-run               # say what is missing; call nothing
./scripts/translate --check                 # exit 1 if any locale is incomplete; call nothing
./scripts/capture-fixtures.sh               # re-capture fixtures, disconnected set (safe)
./scripts/capture-fixtures.sh --connected   # also brings the VPN up and back down
./packaging/appimage/build.sh               # AppImage
./scripts/release.sh                        # dispatch the release workflow (logged-in `gh` only)
./scripts/release.sh --dry-run              # print the dispatch that would be sent, and stop
./scripts/release.sh --print-version        # the tag a release from this checkout would publish
./scripts/release.sh --local --dry-run      # build and package a release without publishing
```

CI runs exactly these gates — fmt, dependencies, translations, clippy, tests — and `main` cannot
move until they are green. The translation step is not a fifth rule: `build.rs` already refuses to
compile an incomplete locale, so clippy and the tests would fail anyway. It runs first because it
names what is missing and the one command that fills it, in a second, instead of leaving a wall of
build-script output to read. It calls no model and reads no key.

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

A release is asked for, not a consequence of `main` moving: `.github/workflows/release.yml` has no
`push` trigger, and `./scripts/release.sh` dispatches it with `gh workflow run --ref main` using the
`gh` login already on the machine. The workflow then publishes version `X.Y.N`: `X.Y` is the latest
release tag, `N` is the commit count of `main`. Nothing is bumped by hand, and a release that waits
for three merges simply skips the numbers in between.

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

act pull_request      -W .github/workflows/ci.yml -P ubuntu-24.04=pvpn-act:24.04 --pull=false
act workflow_dispatch -W .github/workflows/release.yml -P ubuntu-24.04=pvpn-act:24.04 --pull=false -s GITHUB_TOKEN=
```

With an empty `GITHUB_TOKEN` the release script stops after packaging (the publish job calls it
with `--skip-build`, which is its local mode — `gh workflow run` is never reached there), so `act`
exercises everything except the upload. The provenance attestation needs GitHub's OIDC endpoint and
can only be checked by a real run.

## Conventions

- Rust 2024; `rustfmt` defaults; clippy clean under `-D warnings`.
- Commit messages: conventional-commit prefix, then prose explaining **why**. `git log` is the
  template — the existing messages are deliberately detailed because the reasoning is the part
  that does not survive in the code.
- `protonvpn-core` must never depend on a GUI toolkit.
- Prefer making illegal states unrepresentable over checking for them at runtime.
- Comments explain reasoning and measured facts. Not what the next line does.

## Language

**English is the source language.** Every word the application says to a person lives in
`crates/protonvpn-core/i18n/en/*.ftl` — Project Fluent, one directory per language, a `#` developer
comment above every message saying where it appears and how much room it has. That comment is why
Fluent was chosen over a table of constants: it is the translator's only view of the screen.

The whole contract is [`docs/i18n.md`](docs/i18n.md), and `build.rs` enforces it: a locale that is
short a message, a file or a `$variable` **does not compile**. That is deliberate — a runtime
fallback to English is the failure that goes unnoticed for a year.

> **Before opening a pull request, run `./scripts/translate`.**

It reads `$OPENAI_API_KEY`, translates exactly what is missing with `gpt5-terra` (override with
`$TRANSLATE_MODEL`), writes it with the English comment carried along, and finishes by running the
real check. `./scripts/translate --check` reports gaps without calling anything.

Adding a language is adding a directory: `mkdir crates/protonvpn-core/i18n/de`, run the script. The
`Locale` enum, the language picker in Settings and the environment detection all follow from the
directory tree.

Two rules that are easy to get wrong and expensive later:

- **A literal is not a string.** If it is not in the catalogue it cannot be translated, and a
  `format!` that builds a sentence out of Russian fragments is a sentence in one language forever.
  Grep for Cyrillic before you finish: `grep -rnP '[\x{0400}-\x{04FF}]' crates --include='*.rs'`.
  Exactly one hit is legitimate, and it says so in its own comment: the test in
  `views/settings.rs` that asserts the Russian wording of a plural whose English has two forms and
  whose Russian has three — the one place a copy of the English would still compile and still be
  wrong.
- **Data is never translated.** A server name, a country, a city, an IP address, a port, a version,
  a path, a URL, an exit code, a CLI key or value, and the command line itself are facts about the
  outside world. Our own words *around* them are what the catalogue holds — the console exists to
  show the CLI's bytes exactly as they arrived, and that is the whole reason it is trustworthy.

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
  countries for the same IP* — see `docs/cli-surface.md` §4.9. Country and provider are shown
  because they are worth reading; nothing in the application is decided by them, and nothing is
  decided by comparing two readings either (`docs/architecture.md` §8).
- **The CLI refuses to run while the official GTK app is running**, because it checks for the
  session-bus name `proton.vpn.app.gtk`. Our app must never own that name.
- **`protonvpn signin` prompts on a TTY**, which is why every invocation goes through a PTY
  rather than a pipe. One code path, not two.
- **Capturing fixtures and connecting both change this machine's network.** Ask a human first.
- **`docs/research.md` records two retracted conclusions** — NetShield breaking on client exit,
  and GeoIP consistency checking — kept deliberately, with the evidence that overturned them. Do
  not re-derive the old conclusions from the surrounding text.
- **The SOCKS5 proxy pins the route and holds it; it does not demand that the route changed.** It
  opens when the CLI reports a connection, pinning the source address the kernel answers with, and
  closes — dropping what was relaying — the moment that address moves, vanishes or stops carrying
  anything. Do not re-introduce a comparison against a "before connecting" route: an application
  started while the VPN is already up has only ever seen the tunnel's own, and the comparison is
  what used to keep the proxy **shut** about a perfectly good tunnel (`docs/architecture.md` §13).
- **State must be shown with its age** (`updated 3 mins ago`), never as a bare verdict and never
  with the word "stale". See `docs/architecture.md` §7.
- **The AppImage's file name is a contract.** `ProtonVPN-GUI-<version>-<arch>.AppImage` is how the
  updater learns the version — it reads it back out of `SHA256SUMS`, and there is no GitHub API in
  the picture. Rename the asset in `packaging/appimage/build.sh` and the updater stops finding
  releases; `scripts/release.sh` refuses to publish an image whose name disagrees with the version
  baked into it.
- **winit cannot hide a window on Wayland** — `set_visible` is literally "Not possible on
  Wayland". "Close to tray" is therefore destroy-and-recreate, and the app must be an
  `iced::daemon`: an `iced::application` exits the moment its last window is destroyed, which
  would make closing the window quit the app.
- **iced 0.13 leaves a ghost window on Wayland; 0.14 does not.** 0.13's `iced_winit` created a
  throwaway "winit window" to boot the compositor, and `tiny-skia`'s `softbuffer::Context` kept it
  alive — visible in Alt+Tab and counted by the dock, invisible on screen. iced 0.14 boots the
  compositor lazily on the first real window (iced-rs/iced#2722). Do not pin back to 0.13; §12.1
  of `docs/architecture.md` has the measurement.
- **Wayland has no window icons.** The window's name and icon come from a `.desktop` file whose
  basename matches the window's app id, which is why the app installs one into
  `~/.local/share/{applications,icons/hicolor}` and why `desktop::entry()` and
  `packaging/protonvpn-gui.desktop` are kept identical by a test — `Exec` and `Icon` aside
  (`docs/architecture.md` §12). The installed `Icon=` is an absolute path on purpose: an icon
  *name* is a theme lookup, and GTK answers a lookup of the user's hicolor directory from a stale
  `icon-theme.cache` that nothing rebuilds, which is how the gear comes back.

## Current state

**Done:** research; architecture; workspace; the PTY driver (`src/pty.rs`); the capture harness
(`src/bin/capture_fixtures.rs`, `scripts/capture-fixtures.sh`); a frozen PTY fixture corpus with
guarantee tests. Then Phase 1 and most of Phase 2: `runner.rs`, `logbus.rs`, `interpreter.rs`,
`parse.rs`, `launcher.rs`, `poll.rs`, `probe.rs`, `net/natpmp.rs`, `config.rs`
and `engine.rs`, plus the `iced` window, the console pane, the `ksni` tray and autostart. Then the
project's own plumbing: protected `main`, CI on every pull request, and a release pipeline. Then
the connection manager and the redesigned window ([`docs/architecture.md`](docs/architecture.md)
§11): country, city, P2P, Secure Core, Tor and port forwarding are properties of a **saved
connection**, the shell is a light two-page window (Overview / Settings) with the console pinned
underneath, and the login form is a page rather than a tab. `design/after/` holds screenshots of
the result. Then a dependency audit: iced's unused `auto-detect-theme` — and behind it
`dark-light`, a second `zbus` stack, `dconf` and a desktop-sniffing crate — is gone, taking the
Linux closure from 241 crates to 218 and `Cargo.lock` from 384 entries to 321, and
[`scripts/check-linux-deps.sh`](scripts/check-linux-deps.sh) now holds the line. Then the release
pipeline: a release publishes an AppImage alongside the tarball, assembled in a read-only job
from a toolchain pinned by version and SHA-256, so the third-party `linuxdeploy`/`appimagetool`
never runs in the job that can write. Then the trigger itself: the workflow is dispatch-only, and
`./scripts/release.sh` asks for a release with `gh`, so a merge and a release stopped being the
same decision. Then two fixes that came out of running the app on GNOME: iced went from 0.13 to
0.14, which removes the ghost "winit window" from Alt+Tab and takes the Linux closure from 218
crates to 195, and the app now installs its own `.desktop` entry and icon into `~/.local/share`,
which is the only way a Wayland desktop can give the window a name and an icon at all
(`docs/architecture.md` §12). Then an opt-in qBittorrent port push — the third sanctioned
exception — was built and removed again: it never worked against a real client, and it is not
worth a standing hole in the "only `protonvpn`" rule ([`docs/architecture.md`](docs/architecture.md)
§0, §10.4). Then a bug from live use: with `Connect at startup` on, the app re-issued
`connect` on every start even when the CLI already reported a connection — and `connect` against a
live tunnel switches servers silently (`docs/cli-surface.md` §4.4), so a working tunnel was being
rebuilt on launch. The startup connect is now a *request* the engine holds until the session's
first `status` answers, and stands down if it does
([`docs/architecture.md`](docs/architecture.md) §11, rule 5); a manual Connect is untouched. That
surfaced a second bug underneath: invocation ids were minted twice — by the engine for a job it had
queued, by the log bus when the record opened — so anything recorded in between (a `curl` reading,
a NAT-PMP renewal) could swap ids with a waiting command and file that command's output under the
note, where the interpreter would read it as the note's; the bus now takes the id the engine
reserved (§3). Then the paranoid option ([`docs/architecture.md`](docs/architecture.md) §13):
`socks5.rs` is a loopback-only SOCKS5 proxy, off by default, whose gate holds the route the kernel
answers with, whose watchdog re-reads that route every 200 ms without a packet, and whose
`net/route.rs` needs no NetworkManager, no D-Bus and no Proton file to answer the only question it
asks.

Then the AppImage updater (exception #4, [`docs/architecture.md`](docs/architecture.md) §14):
`scripts/version.sh` is the one definition of `X.Y.N`, baked into the binary by
`crates/protonvpn-gui/build.rs` and printed by `--version`; `crates/protonvpn-core/src/update.rs`
reads the latest release out of `releases/latest/download/SHA256SUMS` (no API, no rate limit, the
version inside the asset's own name), checks the bytes against the checksum published with them and
against the type-2 AppImage marker, and swaps the image in place with a hard link and an atomic
rename — the previous one stays as `<name>.old` until the next start proves the new one works. The
check and the download run off the engine's thread and off the runner's queue, four policies govern
what happens without being asked (`download` by default), and **nothing downloaded is ever
executed**: the new image takes effect at the next start. What the checksum does not prove —
authorship — is written down in `SECURITY.md` rather than implied.

Then localization ([`docs/architecture.md`](docs/architecture.md) §15,
[`docs/i18n.md`](docs/i18n.md)): the application had exactly one language and it was not the one in
the source — every sentence was a Russian literal in a Rust file, so a second language would have
been a fork. Now English is the source and the words live in a Project Fluent catalogue under
`crates/protonvpn-core/i18n/`, one directory per language; `build.rs` parses all of it **before the
crate compiles** and refuses to build a locale that is short a message, a file or a `$variable`, so
an incomplete translation is a compile error rather than a silent fallback. The same script
generates one typed method per message, which makes a forgotten argument a compile error too. Each
message carries a developer comment saying where it appears and how much room it has — the reason
Fluent was chosen over a table of constants — and `scripts/translate` fills the gaps with an LLM,
sending only what is missing and finishing by running the real check. The language follows the
desktop (`$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG`) and can be pinned in Settings → General.
Our words are translated; the CLI's bytes are not.

Then two halves of one bug that only a login shows. The tray item was created by a `spawn()` that
required `org.kde.StatusNotifierWatcher` to have an owner at that instant, and an autostart run is
routinely up before the shell's panel is — so no item was created and the session had no tray at
all. The item is now built with `assume_sni_available(true)` and whether a panel has it is read live
from ksni's own watcher callbacks, so it registers whenever the panel appears; and a run that asked
to live in the tray waits five seconds for one before it opens the window instead
([`docs/architecture.md`](docs/architecture.md) §9). The other half is what that state left behind:
with no tray the close button refuses on purpose, and the notice it showed pointed at a tray menu
that was not there — so the window now carries its own **Quit**, in the sidebar and on the login
page, from the same catalogue entry the tray menu uses.

Then a bug from the same kind of live use: the GUI was restarted while the VPN stayed up, and the
SOCKS5 proxy never opened. It armed only on a route the kernel had been *seen to change* while the
CLI said the tunnel was down — and a process that starts with the tunnel already up has only ever
seen the tunnel's own route, so it landed in `route … is unproven` and stayed there until a
reconnect. The egress reading had the same shape: the first address it ever read became the
"baseline", so the next reading matched it and the watch closed the gate as "the tunnel is not
carrying traffic" about a tunnel that was. Both comparisons are gone. The gate now pins whatever
the kernel answers when the CLI reports a connection and holds it — every dial and a 200 ms
watchdog re-read it — while a route that breaks is not re-pinned on the CLI's word alone: the
tunnel has to be reported gone first, which is what a reconnect does. The probe keeps one reading
instead of two, taken at startup, on every connect and disconnect, on demand and on the watch's
own clock, and shows it as `Current IP` / `Current country` / `Provider` with its age; whether the
address moved is the user's to notice. The cost is written down rather than hidden: a tunnel that
is routed but carries nothing is no longer detected (`docs/architecture.md` §13.2).

**Next:** a live `signin` run with real credentials (needs a human — the password prompt is
captured, the 2FA prompt is not); a live port-forwarding check against a P2P server; a live
SOCKS5 round trip with a real application on the other end of the listener; desktop
notifications, which need a human decision because they would be a new sanctioned exception.

**Verified live** (2026-10-05, the updater): the release page answers and the answer is parsed into
a release or a finding with a reason; a real 4.9 MB asset is downloaded through the redirect to
`objects.githubusercontent.com`, its size survives that redirect, it hashes to the checksum the
release published (checked again with `sha256sum`), it passes the type-2 check, and it is swapped
in place with the previous image kept as `.old` and removed at the next start. Replacing the file
under a **running** AppImage is invisible to it — the FUSE mount keeps serving the old bytes, the
window redraws the same frame, and a fresh start runs the new image. Run headless with a sandboxed
`$HOME`, the application itself performed its scheduled check and wrote `last_check` into its
config. The repeatable version of all of that is
`cargo test -p protonvpn-core --test live_update -- --ignored`.

**Verified live** (2026-10-08, the autostart tray race): on a private session bus with a mock
`org.kde.StatusNotifierWatcher` and a headless `sway`, a watcher that appeared two seconds *after*
the application was registered by the new build and never by the old one — the old one printed
`tray-unavailable` and opened a window instead, which is the reported bug reproduced exactly. With
no watcher at all the window appears after the five-second wait, with the notice in it and its own
Quit in the sidebar; with the watcher there before the application, no window is ever opened. The
window was photographed on `sway`'s Xwayland: `xwininfo -root -children` gives the id and
`ffmpeg -f x11grab -window_id <id>` the picture — grabbing the root window comes back black,
because rootless Xwayland never composites the children into it.

**Verified live** (2026-09-30, two full rounds): the app connects on start to the configured
country and the tray reports it, `protonvpn status` agrees, the egress address changes and comes
back, disconnecting from the tray menu takes effect within seconds, quitting from the tray leaves
no window and no bus name, and no NetworkManager profile or egress change is left behind.

**Verified live** (2026-10-09, the proxy gate on a restart): the tunnel really was up — `ip route get
1.1.1.1` answered `dev proton0 … src 10.2.0.2` — and the application was started headless (`sway`
with `WLR_BACKENDS=headless`, a sandboxed `$HOME`, the proxy enabled on a spare port) as if the GUI
had been restarted while the VPN stayed up. The CLI was a stand-in reporting `Status: Connected`,
because the real one cannot write its log or its runtime lock under the read-only `/` of this
sandbox; the kernel's route and the relay were the machine's own. The **old build** bound the
listener and refused every dial with SOCKS5 `0x02` — `curl: (97) cannot complete SOCKS5 connection
… (2)` — which is the reported bug reproduced exactly: the route it had sampled at startup was the
tunnel's own, so nothing was ever "proven". The **new build**, in the same situation, pinned
`10.2.0.2` and relayed: `curl --socks5-hostname 127.0.0.1:<port> https://ifconfig.co/json` came back
with an egress address, and every dial it served was re-checked against the pin.
