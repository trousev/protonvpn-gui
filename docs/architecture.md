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
| 3 | qBittorrent Web API on localhost | optional convenience: hand the forwarded port to the P2P client so the user never copy-pastes | **off by default**, localhost only, user-enabled (§10.4) |

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

### 10.4 Port delivery — **RESOLVED: display, plus opt-in qBittorrent push**

Two things, deliberately separated:

**Always:** the forwarded port is displayed prominently with a copy button. It must be pasted
into whatever P2P client the user runs, so copyability is a core requirement, not a nicety.

Whether a lease is held at all is a property of the connection being connected (§11), not of the
application. **Opt-in, off by default:** a **separate tab** holds an optional checkbox for pushing
the port into qBittorrent via its Web API. Disabled unless the user explicitly turns it on — enabling it
is a deliberate act, because it changes another application's configuration.

Tab contents:

- enable checkbox — **off by default**
- host and port, defaulting to `localhost:8080`
- optional credentials (qBittorrent is commonly configured to bypass auth for localhost)
- behaviour: on every port change, `POST /api/v2/app/setPreferences` with `listen_port`

Design notes:

- **Localhost only.** Never a remote host. This is a local convenience integration, not a
  feature for managing a remote client.
- **Every push is visible in the console.** The console is where we show what we did, and an HTTP
  call is still something we did. Render it as an honest pseudo-invocation, e.g.
  `POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949} → 200 OK`.
  It is not a `protonvpn` command and must never be dressed up as one.
- Credentials storage is a detail to settle when it is built: a credential for a third-party
  local service does not belong in plaintext config, and the §0 prohibition covers only Proton's
  keyring entries, not the system keyring as such.
- This is exception #3 in §0, and it stayed the smallest of the three: it runs nothing, it talks
  to localhost, and it is off until asked for.

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

The window itself is two pages and a console. **Overview** is status, the ground-truth probe and
the connection list; **Settings** is `config list` grouped into tabs, plus the handful of settings
that are ours (autostart, start hidden, connect at startup, the probe). A key the CLI grows that
we have never seen is still shown, under its own name, in «Общие» — hiding it would be a lie of
omission. Signed out, the whole window is the login page: there is nothing else that can honestly
be done until `protonvpn info` names an account.

The console stays pinned under both pages, collapsed to one line: runner status, connection
status, and the age of that knowledge (§7). It is still the product.

