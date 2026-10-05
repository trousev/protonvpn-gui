# Architecture

Authoritative design. Where this document and [`plan.md`](plan.md) disagree, this one wins;
plan.md is the roadmap, this is the contract.

---

## 0. The principle

**The console is the product.** Everything else is either a view onto it or a reducer over it.

The application is a wrapper, and it **does not hide that it is a wrapper**. The user can always
see the exact command that was executed and its exact output. When the console is collapsed, the
collapsed state still says what is happening — working or idle.

Second principle, equally load-bearing:

> **We do not have the right to know how the connection works.**
> The only program we ever execute is `protonvpn`. If the CLI says it is connected, it is
> connected. NetworkManager, D-Bus, the gateway, Proton's internal files — none of it is our
> business. It is called internal for a reason.

### Sanctioned exceptions

Everything outside `protonvpn` that we are permitted to touch. Each is narrow, each is bounded,
each exists because the CLI genuinely cannot do the job.

| # | Exception | Why it exists | Bounds |
|---|---|---|---|
| 1 | `curl` to an IP-echo service | ground truth: the CLI's self-report is unreliable (measured: it printed `149.88.27.213` while real egress was `149.22.89.89`) | read-only, third party, keyless, no Proton data involved |
| 2 | NAT-PMP to `10.2.0.1:5351` | the port-forwarding lease — the CLI only sets a preference and tells the user to run an external script | gateway is publicly documented by Proton, port is an IANA standard (RFC 6886); probe with opcode 0 first, degrade honestly |
| 3 | A local SOCKS5 listener, and the kernel's route answer behind it | an application that must never touch the network without the VPN needs a door that closes by itself; nothing in `protonvpn` provides one | **off by default**, loopback only, IPv4 + `CONNECT` only, fails closed on evidence rather than on hope (§13) |
| 4 | `curl` to our own release page | an AppImage has no package manager behind it, and the file that would have to be replaced is the one currently running | two URLs, both ours — the `SHA256SUMS` of the latest release and the asset that file names — https only; the bytes are checked against that file; **nothing downloaded is ever executed**: the new image takes effect at the next start (§14) |

The updater is the newest of them and the least entangled with the VPN: it speaks to nobody but
this project's own release page, it has never heard of Proton, and it can change nothing about a
connection. It is still an exception, because it runs a program that is not `protonvpn` and writes
a file that will later be executed — so it is written down, bounded, and off the command path (§14).

An exception was tried and withdrawn: an opt-in push of the forwarded port into a local
qBittorrent over its Web API. It never worked against a real client, and a convenience for one
torrent client is not worth a permanent hole in the "only `protonvpn`" rule. The port is
displayed and copyable instead (§10.4), which is what the feature was for.

**Forbidden, permanently:** NetworkManager, D-Bus, the keyring's Proton entries, the gateway
except as listed above, `/run/user/$UID/Proton/VPN/forwarded_port`, and Proton's `settings.json`
and `app-config.json`.

---

## 1. Layers

```
        ┌──────────────────────────────────────────────────┐
        │  VIEWS                                           │
        │    main window  ·  tray  ·  console pane         │
        └──────────────────────────────────────────────────┘
                 ▲                            ▲
                 │ state                      │ raw lines
                 │                            │
        ┌────────┴─────────┐        ┌─────────┴────────────┐
        │  INTERPRETER     │◄───────│  LOG BUS             │
        │  (pure reducer)  │  lines │  (verbatim, ordered) │
        └──────────────────┘        └─────────▲────────────┘
                                              │ lines + exit code
        ┌──────────────────┐        ┌─────────┴────────────┐
        │  LAUNCHER        │───────►│  RUNNER              │
        │  intent → argv   │        │  serialized children │
        └──────────────────┘        └─────────┬────────────┘
                                              │ exec
                                        ┌─────┴─────┐
                                        │ protonvpn │
                                        └───────────┘

        ┌──────────────────────────────────────────────┐
        │  GROUND TRUTH PROBE   (exception #1)        │
        │  curl <ip-echo service>                      │
        └──────────────────────────────────────────────┘

        ┌──────────────────────────────────────────────┐
        │  APPIMAGE UPDATER     (exception #4)        │
        │  curl SHA256SUMS → curl image → rename       │
        │  (never executed; the next start is the new  │
        │   build, and the old one is kept until then) │
        └──────────────────────────────────────────────┘
```

Data flows one way. Views never call the runner. The launcher never reads output.

---

## 2. Runner — the only thing that executes anything

Responsibilities:

- Spawn `protonvpn …` as a child process.
- **Serialize**: strictly single-flight. Two concurrent `protonvpn` processes fight over the
  same state, so a second request queues rather than running.
- Capture the **full invocation record**: `argv` verbatim, working directory, start/end
  timestamps, exit code, and every line of stdout and stderr — merged into one ordered stream.
- **Stream**, do not buffer. Lines reach the log bus as they are produced, so the console is live
  and the interpreter can react mid-command.
- **Hand over an unterminated line once the child goes quiet.** `signin` writes `Password: ` with
  no newline and then blocks; a strict line reader sits on it forever and the login hangs with no
  way in and no way out. A tail that has been silent for ~200 ms is a prompt, not half a line.
  Measured — see [`cli-surface.md`](cli-surface.md) §1.
- **A running command can be interrupted.** An interactive child has no deadline by design (it is
  waiting for a human), so the console offers «Прервать»: waiting must not be the same thing as
  being stuck.
- Expose runner status for the collapsed console:

```
RunnerStatus = Idle | Running { argv, started_at } | Queued { depth }
```

Note this is **runner** status ("working / idle"), which is independent of **connection** status.
A collapsed console must be able to show "working: `protonvpn connect --country uk`" while the
connection state is still `Connecting`.

---

## 3. Log bus

One source of truth, two consumers. The console pane and the interpreter read **the same stream**,
so they can never disagree about what the CLI said.

- Ordered, append-only, with a bounded ring buffer for scrollback.
- Every entry carries: timestamp, the invocation it belongs to (argv), the stream (stdout/stderr),
  and the raw text **exactly as received**.
- Raw text is never rewritten, reflowed, or re-parsed in place. Formatting happens at render time.
- Invocation ids are allocated by the engine when it *decides* to run something, and handed to the
  bus when the record opens — the two moments differ, because a job queued behind a running one has
  an id before it has a record. `LogBus::begin` therefore takes an id instead of minting one:
  with two counters, anything recorded while a job waited (a `curl` reading, a NAT-PMP renewal)
  could take that job's number, after which the job's output was filed under the note and the
  interpreter read the note's output as the command's.

---

## 4. Console pane — the core UI element

VSCode-like, anchored to the bottom of the window, with two states:

**Collapsed** (default when idle): a single bar showing
- runner status — `жду` / `работаю: <command>` / `в очереди: N`
- connection status as last known
- freshness of that knowledge, as an age (`updated 3 mins ago`, §7)

**Expanded**: the console slides up, showing, per invocation, in order:

```
$ protonvpn connect --country uk
Connected to UK#123 in London, United Kingdom.
Your new IP address is 1.2.3.4.
                                              exit 0 · 3.4s
```

Requirements:

- The **command line is shown verbatim**, including flags — never paraphrased.
- Output is shown **verbatim**, ANSI included: it is a real PTY stream (§10.3), so colours and
  progress rendering are the CLI's own, not ours.
- Exit code and duration are shown per invocation.
- Copy button for the whole transcript and per invocation.
- Autoscroll with a "stick to bottom" behaviour, pausable when the user scrolls up.

**The console is read-only.** It is a transcript, not a terminal emulator: no prompt, no input
line, nothing to type into. The app is a wrapper, and its job is to show what it ran — not to
become a shell. Input that the CLI genuinely requires (the `signin` password, a 2FA code) is
collected through purpose-built masked fields and fed to the child over the PTY; secrets never
enter the transcript.

---

## 5. Interpreter — the reducer

A **pure function**:

```
interpret(state: AppState, event: LogEvent | InvocationFinished) -> AppState
```

It reads the log bus and updates internal project state. It owns:

| Domain | Set by parsing |
|---|---|
| connection | `Status: Connected` / `Disconnected` + `Server:` / `Load:` / `Protocol:` |
| account | `protonvpn info` → `Account: '<name>'` |
| countries | `countries list` table |
| cities | `cities list <CC>` table |
| settings | `config list` table |
| errors | `Error: …` lines, plus non-zero exit codes |

Rules:

1. **Never guess.** An unrecognised line does not produce state. Unknown output is preserved as
   raw text and shown, and the affected state stays as it was, marked stale.
2. **Never invent a status.** Absence of information is `Unknown`, not `Disconnected`.
3. The interpreter is testable in isolation — it takes text, it returns state. All Phase 0
   fixtures under `crates/protonvpn-core/tests/fixtures/` are its test corpus.
4. Parsers are strict about shape and tolerant about noise: they skip progress chatter such as
   `Server list is outdated, updating...`.

---

## 6. Launcher

Maps an **intent** to an **argv**, and nothing more.

```
Connect { country: "uk" }   -> ["protonvpn", "connect", "--country", "uk"]
Connect { city: "Zurich" }  -> ["protonvpn", "connect", "--city", "Zurich"]
Connect { fastest: true }   -> ["protonvpn", "connect"]
Connect { server: "IT#23" } -> ["protonvpn", "connect", "IT#23"]
Disconnect                  -> ["protonvpn", "disconnect"]
RefreshStatus               -> ["protonvpn", "status"]
ListCountries               -> ["protonvpn", "countries", "list"]
ListCities { country }      -> ["protonvpn", "cities", "list", country]
ListSettings                -> ["protonvpn", "config", "list"]
SetSetting { key, value }   -> ["protonvpn", "config", "set", key, value]
AccountInfo                 -> ["protonvpn", "info"]
```

The launcher **does not interpret results**. It does not decide whether a connect succeeded. It
hands the argv to the runner and gets out of the way; the interpreter finds out independently by
reading the log, and by the status poll the launcher triggers on completion.

This is what makes the design honest: the launcher's only claim is "I ran this command".

---

## 7. Polling policy

- **At startup, before anything else.** The first `status` of a session is also the reading the
  startup connect waits on (§11): the app never asks for a tunnel before it knows what is already
  there, and it makes the request before the answer arrives, so the request is held rather than
  assumed.
- **Idle cadence: no more often than once per 5 minutes.** At a measured ~1 s per `status` call
  this is a ~0.3% duty cycle — the cost objection disappears at this interval.
- **Immediately after any invocation completes**, if that invocation could have changed
  connection state.
- **Attention-driven**: a fresh `status` when the window is opened or the tray icon is clicked.
  The user is looking, so the answer should be current. This is not a timer and does not violate
  the idle cadence.

Consequence to be honest about: **state can be up to 5 minutes stale.** Therefore:

- Every piece of state carries a **timestamp**.
- Freshness is shown as an **age, not a verdict**: `updated 3 mins ago`, `updated 12 mins ago`.
  **Never the word "stale"**, never a warning icon, never a nag. The user can see the age and
  judge for themselves; that is the whole point of showing it.
- Ages update live, so the number climbs visibly as knowledge gets older.
- This is the project's principle applied to time: never hide what is actually known, and never
  dress it up either.

Wording, for consistency across the UI:

| Elapsed | Rendered |
|---|---|
| < 10 s | `updated just now` |
| < 60 s | `updated Ns ago` |
| ≥ 60 s | `updated N mins ago` |
| ≥ 60 min | `updated N hours ago` |

---

## 8. Ground-truth probe — exception #1

The one thing besides `protonvpn` we are allowed to run:

```
curl <ip-echo service>
```

Purpose: obtain facts **de facto** rather than from self-reports.

What it is genuinely good for:

- **Is traffic actually flowing through the tunnel?** Compare the post-connection egress against
  the pre-connection baseline. If the CLI says `Connected` and the probe returns the baseline
  address, the tunnel is not carrying traffic. No CLI output can tell us that.
- The CLI's own claim about the egress address is not usable: `Your new IP address is …` reported
  `149.88.27.213` while actual egress was `149.22.89.89`. The probe is the authority.
- **IPv6 leak check**, by querying the two address families separately. Verified working on
  `NL#662` (`2a02:6ea0:c041:6652::34`, a Proton/Datacamp prefix, distinct from the pre-VPN
  baseline `2001:bb6:582:5558:…` → no leak). Note IPv6 support varies by server: an IPv6 request
  to `NL#450` failed outright.

> [!IMPORTANT]
> **Geolocation from the probe must never be treated as a correctness signal.**
> Measured: the single IPv4 address `205.147.16.120` was reported as country **NL** by
> `ipinfo.io` and country **US** by `ifconfig.co` — both being Proton infrastructure. The GeoIP
> databases simply disagree. An earlier idea for a "geo consistency check" against the CLI's
> `Server: … in <city>` line is therefore **dropped**: it would raise false alarms.
> Country and city are worth *displaying*; they are not evidence.

Comparison rule: baseline and post-connection readings must come from the **same** endpoint, so a
database disagreement can never masquerade as a state change.

Candidate endpoints (all keyless), returning JSON. Use a small fallback chain:

| Endpoint | Fields | Notes |
|---|---|---|
| `https://ifconfig.co/json` | `ip`, `country_iso`, `asn`, `asn_org` | richest; ASN is the most useful field |
| `https://ipinfo.io/json` | `ip`, `city`, `region`, `country`, `org`, `loc`, `timezone` | rate-limited |
| `https://ifconfig.me/all.json` | `ip_addr`, `user_agent`, `port`, `method` | thinnest |

---

## 9. Tray

The tray is a **peer view**, not a sub-feature of the window:

- **Tray shows connection status only.** Not the runner status. The tray is a glanceable status
  indicator, not a progress monitor.
- Fully functional with no window ever shown. Autostart goes straight to the tray.
- Menu offers connect/disconnect and show/quit.
- Because it must work headless, the interpreter, launcher and runner **must not depend on the
  GUI toolkit** — this is why the core lives in `protonvpn-core` with no UI dependency.
- GNOME without the AppIndicator extension has no tray at all; the app must detect this and stay
  reachable rather than hiding into nothing.

`RunnerStatus` (`жду` / `работаю` / `в очереди`) lives **only in the main window**, in the
collapsed console bar. Note that it is event-driven — it changes when we launch something, not on
the 5-minute poll — so it would not actually flicker. Keeping it out of the tray is still the
right call: the tray answers "am I connected", nothing else.

---

## 10. Decisions

### 10.1 Port forwarding vs the "only `protonvpn`" rule — **RESOLVED: granted**

**Ruling:** port 5351 being an IANA standard (RFC 6886) settles it. NAT-PMP against the
documented `10.2.0.1:5351` is **allowed** — exception #2 in §0.

Port forwarding cannot be done through the CLI: `protonvpn config set port-forwarding on` only
sets a preference, and the CLI's own help says the lease needs an external script.

For the record, the endpoint does **not** come from the CLI, and this was worth checking:

- The CLI mentions NAT-PMP exactly once, in a help string: *"run the natpmpc setup script"*. It
  never emits `10.2.0.1`, never emits `5351`, and `status` / `config list` / `info` expose none
  of it. Verified by grepping the whole CLI package.
- The gateway `10.2.0.1` comes from Proton's **public manual-setup guide**, which contains
  verbatim: `natpmpc -g 10.2.0.1` and
  `natpmpc -a 1 0 udp 60 -g 10.2.0.1 && natpmpc -a 1 0 tcp 60 -g 10.2.0.1`.
  That guide is written for humans to follow by hand — it is a public interface.
- The port is the IANA-assigned NAT-PMP port, and `natpmpc`'s default.

Incidental benefit: Proton's guide warns that `natpmpc` versions `20150609-xxx` contain a bug
that misinterprets the server's response. Speaking the protocol ourselves sidesteps that class of
problem entirely.

Spawning `natpmpc` would not have improved anything — Proton's instructions pass `-g 10.2.0.1`
explicitly, so it carries the identical dependency plus a system package that is not installed.

**Risk containment:** send an opcode-0 public-address request first. If the gateway does not
answer, report "port forwarding unavailable" rather than a wrong port. Nothing else in the app
touches this path, so the blast radius is one optional feature.

### 10.2 Console: read-only or interactive? — **RESOLVED: read-only**

The console is a transcript, not a terminal emulator. No input line. `signin`'s password and any
2FA code are collected in masked fields and written to the child over the PTY; they never appear
in the transcript. See §4.

### 10.3 PTY vs pipe for capture — **RESOLVED: PTY**

`signin` requires a PTY regardless, so there is one code path rather than two.

Consequences to handle:

- Output will contain `\r\n` line endings introduced by the terminal layer — normalise.
- ANSI escape sequences are stripped by the interpreter before parsing. As it turned out, the
  CLI emits **none at all** (verified: 13 captures, `ansi=0` everywhere, even on a TTY) — but
  `strip_ansi` stays, because it is cheap and this is exactly the kind of thing that changes
  without notice.
- Width does not matter: `countries list` is byte-identical at 80 and 120 columns. Set a sane
  fixed width anyway so wrapping stays predictable if that ever changes.
- **Done:** all fixtures re-captured through a PTY by `scripts/capture-fixtures.sh` into
  `tests/fixtures/pty/` (13 invocations, raw output plus `.meta.json`). Details in
  [`cli-surface.md`](cli-surface.md) §4.8.

### 10.4 Port delivery — **RESOLVED: display, and a copy button**

The forwarded port is displayed prominently with a copy button. It must be pasted into whatever
P2P client the user runs, so copyability is a core requirement, not a nicety.

Whether a lease is held at all is a property of the connection being connected (§11), not of the
application.

An opt-in push into a local qBittorrent over its Web API was designed, built, and then
**withdrawn** — see §0. It never worked against a real client, and "hand the port to your torrent
client" is a convenience for one program, not a job the CLI cannot do. Pushing state into another
application also meant carrying a third-party credential, which is a cost the display never had.

What would have to be true to try again:

- a P2P client whose API is actually exercised in a live test, not just in a stand-in HTTP server;
- a story for the credential that is not "hold it in memory and hope";
- the honest console rendering, unchanged: an HTTP call is not a `protonvpn` command and must
  never be dressed up as one.

---

### 10.5 A SOCKS5 proxy vs the "only `protonvpn`" rule — **RESOLVED: granted**

**Ruling:** a loopback-only, off-by-default SOCKS5 listener that relays nothing until the tunnel
can be *shown* to be carrying traffic is **exception #3**. The whole design — the gate, the
watchdog, the dial-time checks, and what it honestly cannot promise — is §13.

The one new system fact it needs is the kernel's own source-address answer for off-link traffic.
That is not "how the VPN works": it is the question every client asks the kernel when it opens a
socket, and all we keep is whether the answer changed.

---

---

## 11. Connections — where a preset lives

A **connection** is a saved spelling of `protonvpn connect`. Everything the CLI lets one connect
decide — `--country`, `--city`, `--p2p`, `--securecore`, `--tor` — belongs to the profile, not to
the application. There is deliberately **no global "default preset" setting** left: the CLI has
connect flags, not defaults, and a "default P2P" switch would have been a global that no command
ever reads.

Two kinds, one selection:

| Kind | Stored? | Editable? | Target |
|---|---|---|---|
| `Fastest`, `Secure Core`, `P2P` | no | no | the CLI's own shortcuts (`connect`, `--securecore`, `--p2p`) |
| user profiles | `Config::connections` | yes | country + city + the three flags + port forwarding |

`Config::selected_connection` holds one id for both kinds (`system:*` or a profile's id). Profile
ids are opaque and stable, so renaming `Работа` does not move the selection; the id is generated
from the name and can never collide with the `system:` namespace.

Rules the code enforces:

1. **The preview is the launcher's.** The editor's argv line is produced by `Intent::Connect(…)`,
   the same function the runner gets, so the sentence the user reads cannot drift from the command
   that runs.
2. **Nothing the CLI does not report.** `countries list` prints a name and a code — no server
   counts, no load, no latency. The picker shows a name and a code; the city list shows the
   features column, which is real. A mock-up's "210 серверов" is not data we have.
3. **Port forwarding is part of the profile.** It stays exception #2 and it stays ours, but the
   *decision* to hold a lease travels with the connection being connected
   (`ConnectTarget::port_forwarding`), never with the application. `protonvpn` has no
   per-connection settings, so a profile that asks for a lease sets the one global preference
   first — `protonvpn config set port-forwarding on` — waits for it, and only then connects. Both
   invocations are in the console, in the order a careful human would run them, and a failed set
   cancels the connect rather than silently holding nothing.
4. **The app never invents a connection.** A selection that no longer resolves (a profile deleted
   by hand, an edited config) falls back to `connect`, and the status line says which connection
   that was.
5. **A live tunnel is never touched on the app's own initiative.** `connect` is not idempotent —
   against a live tunnel the CLI switches servers silently and the egress moves under the user
   (`docs/cli-surface.md` §4.4). So "connect at startup" is a *request*, not a command: it waits
   for the first `status` reading of the session and stands down when that reading already reports
   a connection or one on its way, recording the decision in the console as a note (§10.4) rather
   than as a command that ran. A human asking for a switch goes through `Intent::Connect` and is
   always obeyed; the difference is who asked. Nothing is assumed while the answer is one `status`
   away — assuming "not connected" is the bug, and assuming "connected" would silently drop a
   setting the user turned on.

The window itself is two pages and a console. **Overview** is status, the ground-truth probe and
the connection list; **Settings** is `config list` grouped into tabs, plus the handful of settings
that are ours (autostart, start hidden, connect at startup, the probe). A key the CLI grows that
we have never seen is still shown, under its own name, in «Общие» — hiding it would be a lie of
omission. Signed out, the whole window is the login page: there is nothing else that can honestly
be done until `protonvpn info` names an account.

The console stays pinned under both pages, collapsed to one line: runner status, connection
status, and the age of that knowledge (§7). It is still the product.


---

## 12. The desktop knows us by a `.desktop` file

Wayland has no window icons. There is no `_NET_WM_ICON` to set, `winit`'s `set_visible` is a
no-op for the same reason, and a compositor learns what a window *is* by matching its app id (X11:
`WM_CLASS`) against the basename of a `.desktop` file. With no file to match, GNOME builds a
window-backed application around the window and shows it as «Неизвестное приложение» under the
generic `application-x-executable` icon — the gear. The entry is therefore not branding
decoration: it is the only way the window can have a name and an icon at all.

Three files, all ours, all written where a desktop actually searches:

| File | What it is |
|---|---|
| `~/.local/share/applications/protonvpn-gui.desktop` | the application: `Name`, `Icon`, `Categories`, `StartupWMClass` |
| `~/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png` | the icon `Icon=` points at — a file next to the binary is invisible to the shell |
| `~/.config/autostart/protonvpn-gui.desktop` | autostart: the same entry plus `X-GNOME-Autostart-enabled=true` |

Rules:

1. **The app id is `protonvpn-gui`.** The entry's basename, its `StartupWMClass`, the Wayland
   `application_id` and the icon name all say the same thing, and that is what makes the match
   possible. It must never be `proton.vpn.app.gtk`: the CLI refuses to run while that name is on
   the session bus (§0, `cli-surface.md` §2).
2. **`Exec=` is the running program, spelled properly.** From an AppImage that is the image's own
   path (`$APPIMAGE`), not the temporary mount it was unpacked into — that mount is gone by the
   next login. Paths are quoted by the `Exec` key's own rules, which are not shell rules, and a
   literal `%` is doubled.
3. **The config decides, every start re-asserts it.** `desktop_entry` is on by default, because
   the entry is what makes the window legible at all; turning it off in Settings removes both
   files and they stay gone. The same shape as autostart, for the same reason: a user who deletes
   the file should not be surprised by it coming back unasked.
4. **One template, checked.** `packaging/protonvpn-gui.desktop` is what the AppImage ships and
   `desktop::entry()` is what the app installs; a test compares every key but `Exec` and `Icon`,
   so the menu entry and the window cannot end up branded differently. The two keys it lets
   differ are the two a package cannot know: where the program will live, and where the user's
   home is.
5. **`Icon=` is an absolute path, not an icon name.** A name is a theme lookup, and a lookup of
   `~/.local/share/icons/hicolor` is answered out of that directory's `icon-theme.cache` if one is
   there — a file listing that nothing rebuilds when a file appears underneath it. Measured on the
   maintainer's machine: `icon-theme.cache` dated 2026-09-27, our PNG written by the app on
   2026-10-05, and `Gtk.IconTheme.has_icon("protonvpn-gui")` **false** in a fresh GTK 3 and a
   fresh GTK 4 process, while `steam.png` and the `chrome-*Default` icons in the same directory
   resolved because they predate the cache; deleting or rebuilding the cache made ours resolve
   too — which is what GNOME Shell 50 was showing as the gear, for an app whose entry it had
   already found by name. A path is read directly (`GFileIcon`, not `GThemedIcon`), needs no
   theme, no cache and no cooperation, and is what the entry spec reserves absolute values for.
   The file stays in the hicolor directory: that is where a healthy theme finds it by name, for
   the packaged entry and for anyone else who looks, and the path points at those same bytes.
   The key's escaping is not `Exec`'s — no quoting, and only `\` and control characters are
   written the long way.

### 12.1 Why iced is pinned at 0.14 and not 0.13

iced 0.13 left a ghost window behind on Wayland. `iced_winit` created a throwaway window with
winit's default title — «winit window» — just to bring the compositor up, and the `tiny-skia`
compositor kept that window alive for the life of the process inside a `softbuffer::Context`.
Wayland cannot hide a window, so mutter gave it a `MetaWindow`: an entry in Alt+Tab and a second
dot in the dock, for a window nobody could see. Measured before the upgrade: a tray-only start —
no window of ours at all — still added one Wayland surface, and it went away with the process.

iced 0.14 initializes the compositor lazily on the first real window instead (`iced-rs/iced#2722`,
"…and get rid of the ghost boot window"), so there is nothing left to hide. Do not go back: the
upgrade also took 29 crates out of the Linux closure — `png` with `flate2` and `miniz_oxide`,
`palette`, `rayon`, the `drm` family — and added 6.

---

## 13. The local SOCKS5 proxy — exception #3

An application that **must not reach the network without the VPN** has no way to express that
through this project today, and no way to express it through `protonvpn`: the CLI connects, and
then every program on the machine is on its own. The proxy is the door for exactly that
application: point it at `127.0.0.1:1080` in the application's own settings, and it gets the
system's internet while the tunnel is up, and nothing at all when it is not.

**Off by default. Loopback only. IPv4 only. `CONNECT` only. No authentication.** Disabled until
the user turns it on, because it is a service, and this application does not start services on
people's behalf.

### 13.1 What makes it fail closed

"Fail closed" cannot mean "check `protonvpn status` and hope the check is current" — the CLI costs
a second per call and its self-report is not evidence (§8). So the gate is built on the kernel's
own answer to a question any client asks:

> If an off-link IPv4 packet were sent now, which source address would the kernel use?

That is a connected UDP socket, never written to, no DNS, dropped immediately
([`net/route.rs`](../crates/protonvpn-core/src/net/route.rs)). It reveals nothing about how the
tunnel works — only whether the answer has changed, which is the same evidence the egress probe
uses, moved from the public address to the local route.

| Step | Mechanism | Cost | What it catches |
|---|---|---|---|
| 1 | **The reference.** While the CLI reports the tunnel down — and for three seconds after each such report, so the kernel can withdraw the tunnel's address — the route is sampled and remembered. One more sample is taken at startup, before the CLI has said anything; it runs whether or not the proxy is enabled, because it is four syscalls and a state the user may switch on at any moment. A reference is **withdrawn** whenever the gate is closed on suspicion (steps 4-6): evidence that the route moved under us is spent, and arming again needs a fresh look while the CLI says down | four syscalls, no packets | nothing on its own; it is what turns step 2 into evidence |
| 2 | **The gate opens** only when the CLI reports `Connected` *and* the current route differs from the reference | one route read | a route that was never shown to be the tunnel. If the application starts while the VPN is already up, every route it has seen is the tunnel's own, so the gate stays **shut** and the settings page says why: reconnect once, and it arms |
| 3 | **Every dial** re-reads the route before connecting, and compares the socket's own `local_addr` after connecting — before one byte of the application's is relayed | one route read and one `getsockname`, no packets | the route moving while the dial was in flight (see §13.2 for how wide that window really is) |
| 4 | **The watchdog** re-reads the route every 200 ms while the gate is open; disagreement closes the gate, drops every relayed connection, and withdraws the reference. It is a 200 ms poll, and the engine closes the gate on its next turn — so a new dial is refused at once and what is already relaying dies within about a quarter of a second | four syscalls, no packets | the tunnel going away, without waiting for a `status` poll |
| 5 | **The egress watch** runs the sanctioned `curl` probe (exception #1) every `verify_seconds` (default 30, `0` = off — and nothing at all if the probe itself is switched off in «Опрос») while the CLI says connected: the pre-connection address coming back means the tunnel is not carrying traffic, whatever the CLI says | one HTTPS request | the tunnel that is still routed but no longer passes anything |
| 6 | **Dial failures** that implicate the path — a timeout or an unreachable network, **twice in a row** — close the gate. A relayed connection clears the count, and so does a destination that answered and said no: both prove the path works | nothing | the same case, noticed sooner |

A dial the gate refuses gets SOCKS5 reply `0x02` (not allowed by ruleset); the other replies are
the ordinary ones — `0x07` for a command we do not speak, `0x08` for an IPv6 literal, `0x04` for a
name with no A record, `0x03` for a dial that came from the wrong address. The listener stays bound while
the feature is enabled even when the gate is shut: an application that gets a refusal can say so,
and — the paranoid reason — a port that is released is a port another process can take.

**A gate closed on suspicion re-arms the same way it armed the first time**: the CLI has to report
the tunnel down (so a fresh reference can be taken), and then up, with the route different from that
reference. There is deliberately no "check again" that skips the first half — a different route is
not a tunnel, and a proxy that can be talked into opening on one is not the proxy this section
describes. The settings page says as much, and the remedy is one reconnect.

Two closes do **not** spend the reference: a listener that never came up, and the user switching the
proxy off. Fixing a port number says nothing about the route, and punishing it with a reconnect
would be superstition rather than safety.

### 13.2 What it is not, and what it honestly cannot promise

- **The standard library cannot bind a source address before connecting.** This toolchain's
  `std::net` has no `TcpSocket`, so instead of binding, step 3 checks the address the kernel
  actually used. The window between the two is not microseconds: the name lookup and the connect
  sit in it, up to the dial timeout of ten seconds. If the route moves inside that window, the
  handshake goes out by whatever route exists — the destination address and the machine's real
  source address go with it, the application's bytes do not, and the connection is refused and
  reported as soon as the address is checked.
- **The gate knows one fact: the source address the kernel picks.** A route change that keeps that
  address — another gateway on the same interface — is invisible to it. That is the egress watch's
  job, and it is why the two exist together.
- **DNS is the system resolver's**, after the gate and before the dial, exactly as it would be for
  the application without a proxy. We do not add a resolver, and we do not read `/etc/resolv.conf`.
- **IPv6 destinations are refused** with `0x08` rather than guessed at: a hop the gate cannot pin
  is a hop the proxy does not take. Domain names are resolved to A records at the proxy, so an
  application that sends names never has to know.
- **Loopback destinations, while the gate is open**, are relayed to the loopback address; they
  never leave the machine. A shut gate refuses them too.
- **A destination the tunnel does not carry — the router, a NAS, anything on the local network —
  is refused** (`0x03`), because everything but a loopback address must come from the pinned
  source. The proxy relays to the internet through the tunnel; it is not a way onto your own LAN.
- **No authentication**, because the listener is loopback-only — and that is exactly the promise
  `ssh -D` makes: any local process, including one belonging to another user of the machine, can
  use the door while it is open. Authentication would not change that; it would only move the
  secret onto the same machine.
- **It is not a boundary against a local attacker**, and it is not a firewall. It is one door, for
  one application, that latches itself.
- **Individual connections never enter the console.** A browser would drown the transcript; the
  counters are on the settings page and the lifecycle gets pseudo-invocations, like every other
  thing we do that is not `protonvpn` (§10.4).

### 13.3 Where it lives

`socks5.rs` owns the listener, the protocol and the counters; `net/route.rs` owns the one system
fact. The **gate is written only by the engine** — `open` and `close` are `pub(crate)`, the proxy
reads the gate and reports `Socks5Event`, and §1's "one writer of state" still holds: the proxy's
threads send into the same request queue as everything else, and they cannot open the door
themselves. A close also drops what is already relaying, which is why every path that closes the
gate goes through one of the engine's two methods rather than touching the gate directly.

Two deliberate exceptions to the logging rule, both bounded:

- **The counters** are atomics owned by the proxy's threads, because a per-connection line in the
  console would be noise, and a counter that had to travel through the engine would be a queue
  nobody needs.
- **The background tunnel check** does not write a line per run — two a minute would drown a
  transcript that is supposed to be read. Its *reading* is state like any other: it lands in
  `Egress::current` and the Overview shows it with its age (§7). What the console gets is the
  conclusion: a gate that closed, and why.

---

## 14. The AppImage updater — exception #4

An AppImage is a file. Nothing upgrades it: no package manager owns it, no repository knows about
it, and the file that has to be replaced is the one currently running. Left alone, an installed
image rots — the user learns about a release by reading the releases page, downloads a second
`ProtonVPN-GUI-…-x86_64.AppImage` next to the first, and has to remember which one is which.

So the application updates itself, and this section is the bound it does that within. Three URLs of
our own, one hash, one rename, and **no execution of anything new**:

```
check    curl https://github.com/trousev/protonvpn-gui/releases/latest/download/SHA256SUMS
decide   ── the version in the asset's own name, compared with the one baked into this build
fetch    curl …/releases/download/<version>/ProtonVPN-GUI-<version>-<arch>.AppImage -o <path>.update
verify   sha256 == the one from SHA256SUMS, and the file really is a type-2 AppImage
install  hard link <path> → <path>.old, then rename() <path>.update → <path>
```

### 14.1 What the user is told, and what actually happens

**`off` / `notify` / `download` / `install`.** `download` is the default: an image that never
updates is the problem this exists to solve, and downloading is not installing — the verified file
waits next to the running one, and the user decides when. Every step remains available as an
explicit action whatever the policy says, because a button that refuses to work is a lie; the
policy governs what happens *on its own*.

**Nothing downloaded is ever executed.** No `--appimage-extract-and-run`, no self-restart, no
`exec`. The new image takes effect the next time the user starts the application, and until then
the running process keeps the file it was started from — an AppImage's runtime holds the image
open, so the rename is invisible to it.

**The state is shown with its age, like everything else** (§7): `updated 5 hours ago` next to the
`SHA256SUMS` reading, and a card that distinguishes the four things a status line usually
conflates — *nothing checked yet*, *a release is available*, *a verified image is waiting for a
restart*, and *the new image is on disk while the old build is still the one running*.

**Failures are stated, not retried into noise.** A check that could not answer keeps its error and
its age, and is retried in six hours rather than on the next tick; a download that failed leaves
nothing behind and says why.

### 14.2 How the version is known

The version is `X.Y.N` — `X.Y` the line's base tag, `N` the commit count — and it is defined once,
in `scripts/version.sh`, because two things must agree about it: `scripts/release.sh` publishes the
tag, and `packaging/appimage/build.sh` bakes the same string into the binary. An image that
believed it was `0.1.42` while being published as `0.1.43` would offer an update to itself forever
and never apply one, so the release also refuses to publish an AppImage whose name does not carry
its own version. `protonvpn-gui --version` prints it.

The updater does **not** use the GitHub API. `releases/latest/download/SHA256SUMS` is a permanent
URL that follows to whatever was published most recently, and the version is inside the asset's
name — so the version and the checksum arrive in the same document, there is no rate limit to hit
and no JSON schema of someone else's to track. The price is that the file name is a contract
between the packaging script, the release script and `update.rs`; tests pin it on both sides.

Releases published before the name carried a version — everything up to and including `0.1.20` —
cannot be installed from, because there is no version in the name to read: the updater reports that
it found nothing it could become, which is the truth, and the next release is the first it can act
on.

A build with no baked version — a plain `cargo build` — says so and compares nothing. It is not
treated as older than the latest release, because a development build that guessed would replace
itself with a release.

### 14.3 What the checksum proves, and what it does not

`SHA256SUMS` is fetched over the same connection as the image and from the same origin. That is
worth having: it catches a truncated download, a proxy that injected something, a mirror serving
yesterday's file, and a resumed transfer that went wrong. **It is not proof of authorship.** An
attacker who can answer for `github.com` — a compromised release, a TLS middlebox with a trusted
certificate — can serve both the image and the checksum, and this check would agree with itself.

What closes that gap is the build-provenance attestation the release already carries, checked by
hand:

```sh
gh attestation verify ProtonVPN-GUI-<version>-x86_64.AppImage --repo trousev/protonvpn-gui
```

The honest upgrade would be a signature verified against a key pinned in the binary, which needs no
third party and no network. It is not in this version, and this section says so rather than
implying the checksum is more than it is. `SECURITY.md` repeats it in the place a reader looks for
it.

The second check is the shape: an ELF with `AI\x02` at offset 8, which is what the pinned type-2
runtime produces. A captive portal's error page agrees perfectly with a checksum file served from
the same portal, and is caught by not being an AppImage at all.

### 14.4 The swap

The rename is within one directory and therefore atomic, and the old image is made reachable as
`<name>.old` by a **hard link before** the rename — so there is no instant in which the installed
path is missing a file, not even for the release path itself. The directory is flushed afterwards,
because a rename is atomic but not durable: without it a crash can leave no image under either
name. The file's mode is copied from the one it replaces, and symlinks are resolved first —
replacing a symlink would leave the real image untouched, which is an "update" that reports success
and changes nothing.

The copy is deleted at the next start, and only when the installed image is there: this process is
that image, so it started, so the copy is only disk space — and if the image is missing, the copy
is the last one of anything and this program has no business deleting it.

**Where it cannot write, it says so** — before downloading anything, because "nowhere to put it" is
worth knowing before 70 MB. A root-owned image in `/opt` is a `NotWritable` message and a manual
download; this application never runs `sudo` and never gains a privilege path. A build that is not
an AppImage at all — the tarball, a `cargo run` — reports what the release page says and offers
nothing, because there is nothing it could truthfully offer.

### 14.5 What it deliberately is not

- **Not a delta update.** `AppImageUpdate` and zsync are the ecosystem's answer and they transfer
  less, but they need `.zsync` metadata published beside every release and a third-party AppImage
  to run on the user's machine — a fourth program, and a worse hole than `curl`. If the metadata is
  ever published, that is a separate decision with its own bounds.
- **Not a package manager.** The tarball install is not managed, not tracked and not replaced.
- **Not a notifier.** The tray item is a menu entry, never a desktop notification: notifications
  would be a new sanctioned exception and a new way to interrupt someone, for a fact they can see
  when they look.
- **Never on the command path.** The check and the download live off the engine's thread and off
  the runner's queue; a `curl` in flight has no more to do with `protonvpn status` than the probe
  does. Cancelling kills the child and deletes the partial file — a partly verified image is not
  something to keep.
