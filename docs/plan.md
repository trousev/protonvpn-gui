# Plan — ProtonVPN GUI for Linux, as a wrapper around the official CLI

Companion documents:
[`architecture.md`](architecture.md) — **the authoritative design contract** (console core,
interpreter, launcher, the "only `protonvpn`" rule). Where it disagrees with this file, it wins.
[`research.md`](research.md) — how the official stack actually works (verified on this machine)
[`cli-surface.md`](cli-surface.md) — the exact CLI contract we wrap

---

## 1. Decision

**We wrap `protonvpn` (proton-vpn-cli 1.0.3). We do not reimplement the protocol or the
NetworkManager plumbing. We are a GUI, not a second library.**

Rationale: the official stack is a maintained, GPL-3.0 implementation of a hairy problem
(SRP auth, certificate lifecycle, NM profile management, Local Agent, kill switch). Duplicating
it — even partially — would be a competing library with a fraction of the testing, and it would
have to be reverse-engineered to stay clear of GPL code. Wrapping keeps our code small,
permissive (BSD-2) and honest.

The CLI's stdout is treated as **both the data channel and the debug channel**: every invocation's
raw output is captured, kept, and shown in the UI. When something breaks the user gets the real
tool output instead of a generic error — this is a deliberate feature, not a fallback.

---

## 2. What this buys and what it costs

### Buys

- Connect / disconnect, by country, by city, by specific server ID.
- Presets: fastest, P2P, Secure Core, Tor, random.
- Country and city lists with feature tags.
- Account info; settings read/write.
- Login / logout (via a PTY, because `signin` prompts).
- Tiny codebase, no Python of our own, no protocol drift of our own making.

### Costs — must be visible in the UI, not hidden

1. **`protonvpn connect` exits.** Verified: the tunnel, the IPv6 leak guard, the kill-switch
   profiles and NetShield filtering all survive — they live in NetworkManager or at the gateway,
   which is exactly why we never need to look at them. The only lost duty is **renewing the
   port-forwarding lease**, which the Local Agent used to do in-process. That cannot be done
   through `protonvpn`, so it is **blocked pending the ruling** in
   [`architecture.md`](architecture.md) §10.1. Evidence: [`cli-surface.md`](cli-surface.md) §4.7.
2. **A status poll costs ~1 second** — every call spawns a Python interpreter. This, not the
   protocol, is the real constraint on the UI's responsiveness. Design for event-driven
   updates and a slow idle poll, never a fast fixed timer. Evidence: §4.1.
3. **No per-server browser.** `protonvpn servers` only prints a web link. We get country and
   city granularity, not a load/latency table. Per-server *load* is visible only for the
   currently connected server (from `status`).
4. **Polled state only.** No push notifications.
5. **Runtime dependency** on the official apt package. Packaging must declare it.
6. **Mutual exclusion.** The CLI refuses to run while the official GTK app is running
   (`proton.vpn.app.gtk` on the session bus). Our GUI must use its own app id and surface that
   error clearly.
7. **Port forwarding is P2P-servers-only and paid-plans-only**, and the assigned port is sticky
   within a session but changes after a reconnect. The UI must reflect that rather than implying
   a permanent port.

---

## 3. Architecture

The design contract lives in [`architecture.md`](architecture.md): the console as the core, the
log bus with two consumers, the interpreter as a pure reducer, the launcher that never reads
output, the 5-minute poll policy, and the "the only program we execute is `protonvpn`" rule.
This section only fixes the crate layout that implements it.

```
protonvpn-gui/                        (workspace)
├── crates/
│   ├── protonvpn-core/               lib — no UI dependency (the tray must work headless)
│   │   ├── runner.rs                 spawn `protonvpn …`, stream lines, exit code, serialization
│   │   ├── logbus.rs                 ordered verbatim log, bounded ring buffer
│   │   ├── interpreter.rs            pure reducer: (AppState, LogEvent) -> AppState
│   │   ├── launcher.rs               intent -> argv, nothing else
│   │   ├── parse.rs                  table + status parsers, unit-tested on captured fixtures
│   │   ├── poll.rs                   5-minute idle cadence, immediate poll after a launch,
│   │   │                             attention-driven poll on window/tray interaction
│   │   ├── probe.rs                  `curl` ip-echo ground truth (the one allowed exception)
│   │   ├── pty.rs                    PTY driver for `signin` (password / 2FA prompts)
│   │   └── model.rs                  AppState, ConnectionState, RunnerStatus, Invocation, LogLine
│   └── protonvpn-gui/                bin — views only
│       ├── app.rs                    elm-style update loop
│       ├── console.rs                the collapsible VSCode-style console pane
│       ├── tray.rs                   ksni StatusNotifierItem on its own thread
│       └── autostart.rs              XDG autostart (~/.config/autostart)
└── packaging/appimage
```

`net/natpmp.rs` is the project's second sanctioned exception to "only `protonvpn`", granted in
[`architecture.md`](architecture.md) §10.1: it talks to the gateway address Proton documents
publicly (`10.2.0.1`, standard NAT-PMP port `5351`). Everything else in §0 of that document stays
forbidden — no NetworkManager, no D-Bus, no keyring, no Proton-internal files.

### State model

```
ConnectionState = Unknown | Disconnected | Connecting | Connected{server, location, load, protocol} | Error
RunnerStatus    = Idle | Running{argv, started_at} | Queued{depth}
```

Both carry a timestamp; the UI renders stale state as stale rather than asserting it.
The tray is driven by `ConnectionState`, the collapsed console by `RunnerStatus`.
Because state is polled, transitions we cause are optimistic: the UI shows `Connecting` as soon as
a `connect` child starts, and the interpreter reconciles from the log.

---

## 4. Roadmap

### Phase 0 — Capture the contract — **DONE**

Ran live on this machine, 2026-09-30. Full results in [`cli-surface.md`](cli-surface.md) §4.
Fixtures written to `crates/protonvpn-core/tests/fixtures/`.

Outcome, in one line: **the wrapper model works.** The tunnel and its NM-resident protections
outlive the CLI process; only the Local Agent dies. Key numbers:

- `status` costs **~1 s** (Python spawn) — the dominant UI constraint.
- connected `status` is 4 clean `Key: Value` lines.
- exit codes: `0` success, `2` validation error; `disconnect` and re-`connect` are safe to call
  unconditionally.
- `connect` while connected silently switches servers.
- `disconnect` tears down every NM profile with no leftovers.

Still open (non-blocking): connection-failure exit codes, the `signin`/2FA PTY prompt sequence,
and the CLI's exact error when the official GTK app is running.

### Phase 1 — Tray + window MVP, **including port forwarding** — **DONE**

> Implemented in `crates/protonvpn-core` (`runner`, `logbus`, `interpreter`, `launcher`, `parse`,
> `poll`, `probe`, `net/natpmp`, `qbittorrent`, `config`, `engine`) and `crates/protonvpn-gui`
> (`iced` window, console pane, `ksni` tray, autostart). Deployment is automated up to a release
> AppImage. The live round-trip is **verified** (two rounds): start-to-connect, tray status,
> `status` agreement, egress change and restore, disconnect and quit from the tray, no leftovers.
> Framework confirmed by building it: `iced` + `ksni`, on `tiny-skia` so no GPU is involved.

Port forwarding is in v1 — it is the only genuinely useful feature the official CLI does *not*
cover, and the NAT-PMP work turned out small.

- `iced` window: connection state, one primary connect/disconnect control, country picker.
- `ksni` tray thread: state-coloured icon, menu (Connect / Disconnect / Show / Quit),
  left-click opens the window, window close hides to tray. Fully functional with no window.
- **The console pane** ([`architecture.md`](architecture.md) §4) — collapsible, VSCode-style,
  showing each verbatim command with its verbatim output, exit code and duration. Collapsed, it
  still reports `жду` / `работаю: <command>` / `в очереди: N`. This is the core of the product,
  not a debugging afterthought.
- Autostart `.desktop` + `connect_at_startup` + `start_minimized` (our own config; we do not read
  the official app's `app-config.json`).
- Polling per [`architecture.md`](architecture.md) §7: ≥5 min idle, immediate after any
  state-changing invocation, fresh read on window open / tray click. State carries a timestamp
  and is displayed as an **age** (`updated 3 mins ago`) — never as a verdict, never as a warning.
- The `curl` ground-truth probe (§8): egress IP, geo consistency vs the CLI's claim, IPv6 leak.

**Port forwarding** (allowed, §10.1): NAT-PMP map requests for UDP+TCP with `lifetime=60`,
renewed every 40 s from our own timer — no `natpmpc`, no child process. Send an opcode-0
public-address request first to confirm the gateway is answering; if it times out, degrade to
"port forwarding unavailable" rather than showing a wrong port. Release with `lifetime=0` on
disconnect, show the port prominently with a copy button, and surface `result != 0` as an error.
Respect servers that report "does not support port forwarding" instead of showing an empty port.

**Prerequisite — DONE.** The Phase 0 fixtures were captured through a pipe, so they were
re-captured through a **PTY** before any parser work. Result: 13 invocations in
`crates/protonvpn-core/tests/fixtures/pty/`, produced by `scripts/capture-fixtures.sh`.
Findings were better than feared — the CLI emits **no ANSI at all**, PTY output is **identical to
pipe** output, and the tables are **width-independent** (byte-identical at 80 and 120 columns).
Details in [`cli-surface.md`](cli-surface.md) §4.8.

**Exit criteria:** log in, log out → app is in the tray and connected to the configured country,
no window; clicking the tray opens it; closing returns to the tray; every command the app ran is
visible verbatim in the console with its real output; if the CLI errors, the raw error is
readable and copyable.

### Phase 2 — Actual usability (1–2 weeks) — **mostly done**

> Country/city browser with search and feature tags, presets, server-ID entry, the settings screen
> from `config list` / `config set` (values taken from the CLI's own `--help`), the PTY-backed
> login form with masked fields, and the opt-in qBittorrent tab with its honest pseudo-invocation
> are all in. Desktop notifications are **not**, and deliberately: they would be a fourth
> sanctioned exception. Login has not been exercised against the real prompt sequence, because
> that means signing out first.

- Country/city browser from `countries list` / `cities list`, with feature tags, search, caching.
- Presets: fastest / P2P / Secure Core / Tor / random; direct server ID entry.
- Settings screen driven by `config list` / `config set` (including the port-forwarding toggle).
- Login / logout with a PTY-backed prompt (password + 2FA), including the 2FA-required
  "traffic blocked" case.
- Desktop notifications on state transitions.
- **qBittorrent integration tab** ([`architecture.md`](architecture.md) §10.4): opt-in checkbox
  (**off by default**), host/port defaulting to `localhost:8080`, optional credentials, and a
  `setPreferences` push on every port change. Localhost only. Each push is rendered in the console
  as an honest pseudo-invocation — never dressed up as a `protonvpn` command.
- Flatpak is **dropped** — AppImage is the only packaging target, so none of the sandbox work
  (Background portal, `--own-name`, `--system-talk-name`) is needed. Autostart is a plain
  `~/.config/autostart/*.desktop`.

### Phase 2.5 — Connection manager and the redesigned window — **done**

> [`architecture.md`](architecture.md) §11. Country, city, P2P, Secure Core, Tor and port
> forwarding became properties of a **saved connection** instead of app-wide defaults; the three
> system presets (`Fastest`, `Secure Core`, `P2P`) stay uneditable and unstored. The window is a
> light two-page shell (Обзор / Настройки) with the console pinned underneath, the login page is a
> page rather than a tab, and the settings screen keeps only CLI-reachable keys — a key the CLI
> grows is still shown, one we invent is not. Port forwarding moved with it: the lease is held for
> the *selected profile*, and a profile that wants one sets the CLI's single global preference
> before connecting, in the open.

### Phase 3 — Packaging — **done**

> Releases are automatic: every merge to `main` publishes `X.Y.N` (see `scripts/release.sh` and
> `.github/workflows/release.yml`) with an AppImage, a tarball, a `SHA256SUMS` and a
> build-provenance attestation for each. The AppImage is assembled in a job that can only **read**
> — `packaging/appimage/build.sh` pins `linuxdeploy`, the output plugin that carries
> `appimagetool`, and the AppImage runtime by version, and verifies each by SHA-256 before running
> it — and the publish job only attaches the artifact it is handed. No third-party toolchain ever
> runs in the job that holds `contents: write`. The `.desktop`, icon and `StartupWMClass` are
> shipped in both assets.

### Phase 3 (original notes)

**AppImage only. Flatpak is dropped deliberately** — the sandbox would fight both the host
`protonvpn` CLI and tray-name ownership, for no benefit a wrapper needs.

- AppImage via `linuxdeploy`, built on the oldest glibc we intend to support.
- Declare the runtime dependencies honestly in the UI and the README: `protonvpn`
  (the official apt package). No `natpmpc`, no other system dependency.
- Ship `.desktop`, icon, `StartupWMClass`; autostart is a plain `~/.config/autostart/*.desktop`
  (no portal needed outside a sandbox).
- No `finish-args`, no `--own-name`, no Background portal. Outside a sandbox the app needs
  nothing from the system bus at all — only the session bus, so `ksni` can register its
  StatusNotifierItem directly. (Under Flatpak this was the hard part; dropping it removes the
  whole category of problem.)

### Phase 4 — Removed

An earlier draft proposed observing NetworkManager over D-Bus for push state changes. That is now
**explicitly forbidden** by [`architecture.md`](architecture.md) §0: we do not get to know how the
connection works. Staleness is handled by the timestamp-and-attention-poll design instead (§7).

---

## 5. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| ~~Tunnel or `Connected` status does not survive CLI exit~~ | — | **resolved in Phase 0: it survives** |
| ~~Netshield silently stops being enforced~~ | — | **retracted: NetShield is gateway-side and unaffected** |
| CLI human-readable output changes | parsers break | keep parsers strict, keep raw text, show it, fail loudly; fixtures catch drift |
| CLI refuses to run because the official GUI is open | every action fails | detect, explain, never own `proton.vpn.app.gtk` |
| `status` polling is expensive (~1 s per call) | UI jank, battery drain | **resolved by the ≥5 min idle cadence** (~0.3% duty cycle); immediate poll only after state-changing invocations |
| Concurrent invocations race | flapping connections | single-flight queue |
| `signin` prompts on a TTY | login impossible with piped stdin | PTY driver, tested against the real prompt |
| GNOME without the AppIndicator extension | no tray at all | detect, warn, never hide the window into nothing |
| ~~Flatpak + host CLI~~ | — | **dropped: AppImage only** |
| **NAT-PMP endpoint changes** | port forwarding silently stops | gateway is documented publicly and port 5351 is an IANA standard; probe with opcode 0 first and degrade honestly if it times out |
| TTY changes CLI output vs the captured fixtures | parsers break on colours/progress | **re-capture all fixtures through a PTY before writing parsers** (§10.3) |
| Secrets in the transcript | password leaked into scrollback | read-only console + masked input fields; secrets go to the PTY, never to the log bus |
| State up to 5 min stale | UI asserts something untrue | timestamp every field; render stale as stale, never as fact |
| A connect we did not initiate (user's own terminal) | UI disagrees with reality until the next poll | attention-driven poll on window open / tray click |
| Port shown on a server that doesn't support forwarding | user pastes a port that never worked | only request a lease where the connect output says forwarding is active; otherwise explain |
| Forwarded port changes after a reconnect | user's P2P app points at a dead port | make the current port prominent and easy to re-copy |

---

## 6. Resolved / open

**Resolved**

1. The earlier "NetShield breaks" warning was wrong and is retracted — NetShield is enforced by
   the gateway and works normally. See [`cli-surface.md`](cli-surface.md) §4.7.
2. Packaging — AppImage only, Flatpak dropped.
3. The project is a **CLI wrapper**, explicitly not a second VPN library.
4. Polling — ≥5 min idle cadence; the ~1 s call cost is a non-issue at that interval.
5. The window is not the product — **the console is**, and the tray must work with no window.
6. Staleness is expressed as an age (`updated 3 mins ago`), never as a verdict like "stale".
7. The tray shows **connection status only**; `RunnerStatus` lives only in the main window.
8. Console is **read-only**; `signin` secrets go through masked fields and a PTY.
9. Capture is via **PTY** for every command — one code path.
10. Port forwarding is **allowed** — exception #2; the NAT-PMP endpoint comes from Proton's public
    documentation (`10.2.0.1`, IANA-standard port `5351`), not from the CLI. See §10.1.
11. qBittorrent port push is **allowed but off by default** — exception #3; a separate tab with an
    opt-in checkbox, localhost only, and every push visible in the console as a pseudo-invocation.

**Open**

1. **Framework:** default `iced` + `ksni`; confirm at the deferred UI discussion.
2. Where to store optional qBittorrent credentials, if the user supplies any (a third-party local
   service secret does not belong in plaintext config; the §0 keyring prohibition covers Proton's
   entries, not the system keyring as such).
5. **Framework:** default `iced` + `ksni`; confirm at the deferred UI discussion.
