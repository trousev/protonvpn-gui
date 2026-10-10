# CLI contract — the API our GUI wraps

Verified against `protonvpn` 1.0.3 (`proton-vpn-cli`), Ubuntu 26.04.
This is the exact surface a wrapper may rely on. Everything here was observed, not assumed.

---

## 0. The lifecycle fact that defines the whole design

`protonvpn connect` **exits after connecting.**

Evidence: `proton/vpn/cli/commands/server.py` — the `connect` coroutine ends at the
"Connected to …" echo, and the command is decorated with `@run_async`
(`proton/vpn/cli/core/run_async.py`), which is `asyncio.run(func(...))`.
When the coroutine returns, `asyncio.run` cancels the remaining tasks and closes the loop,
so the process exits. `controller.connect()` calls `disconnect()` **only on failure**.

Consequences — **now empirically verified** (see §5):

| Thing | After `connect` exits |
|---|---|
| NetworkManager WireGuard profile | **survives** — NM owns the tunnel, not the CLI |
| Tunnel traffic | **survives** — verified: egress IP stayed on Proton after exit |
| IPv6 leak guard (`pvpn-killswitch-ipv6` dummy iface) | **survives** — it is an NM connection, not a process |
| Kill-switch NM profiles | survive for the same reason; applied at connect time |
| Local Agent connection | **dies** — an asyncio task in the exited process (verified: no TCP conn to `10.2.0.1:65432`) |
| NetShield | **unaffected** — DNS-level filtering done by the gateway resolver `10.2.0.1`, not client-side |
| Port-forwarding lease | **not renewed** — the port is granted once at connect time, then the lease expires |
| Kill-switch NM profiles | survive for the same reason; applied at connect time |

The rule that falls out of this:

> **Anything implemented as a NetworkManager connection or server-side at the gateway survives.
> Only in-process Local Agent duties are lost — and of those, the only user-visible one is
> port-forwarding lease renewal.**

> [!IMPORTANT]
> An earlier draft of this document claimed NetShield also breaks. **That was wrong** and is
> retracted: NetShield is applied by the gateway, not by the client process (see §4.7).

There is no `--background`, no `--daemon`, no `--json`. Verified via `--help` on every command.

> Our GUI must **poll** `protonvpn status` for state, and must be honest in the UI about the
> Local-Agent-backed features, which stop being maintained once the child process exits.

---

## 1. Commands

### `protonvpn info` — read-only, no auth prompt
```
Account: 'trousev'
```
**Logged-out variant — observed 2026-10-01 on 1.0.3:**
```
Account: 'None'
```
The field is never omitted and never empty: the CLI prints the Python sentinel, quoted exactly
like a real name. This is a parsing trap, not a nicety — read at face value it is an account
called "None", and the GUI believed it was signed in, hid the login page, and left the user with
no way to authenticate and every `connect` failing. The sentinel is therefore mapped to *no name*
in `parse::account`, and `Account: ''` / `Account:` mean the same thing.

### `protonvpn status` — the state source
```
Status: Disconnected
```
Connected output not yet observed (needs a live connection). Docs claim it shows server name,
location, load and protocol. **Must be captured before the parser is written.**

### `protonvpn countries list` — parseable table
```
Server list is outdated, updating... This may take a moment.
Country                           Code
--------------------------------  ------
Afghanistan                       AF
Albania                           AL
...
```
Note the leading progress line — the parser must skip non-table lines. First run may trigger a
server-list download (slow); subsequent runs use the cache.

### `protonvpn cities list <CC|"Full Name">` — parseable table
```

Cities in Switzerland:
City    Features
------  ----------
Zurich  P2P, Tor
```
Features column is comma-separated (`P2P`, `Tor`, `Secure Core`, …).

### `protonvpn servers` — **not useful**
```
Usage: protonvpn servers [OPTIONS]

  View available servers. Prints the link to the full server list on
  protonvpn.com.
```
It only prints a web link. **There is no CLI way to enumerate individual servers or their load.**
So the GUI cannot offer a rich per-server browser with load/latency through the CLI.

### `protonvpn connect [SERVER_NAME]` — one-shot

```
--country TEXT      country code (US, GB) or full name
--city TEXT         city name (quote multi-word)
--p2p               fastest P2P server
-sc, --securecore   fastest Secure Core server
--tor               fastest Tor server
--random            random available server
```
Success output:
```
Connected to <server_name> in <location>. 
Your new IP address is <ipv4>.
```
plus capability lines and an OpenVPN warning if applicable.
Failure → non-zero exit with a message on stdout/stderr.

### `protonvpn disconnect` — one-shot
Calls `controller.disconnect()` then `wait_for_current_tasks()` (comment in source: waits for the
post-disconnect kill-switch notification).

### `protonvpn signin USERNAME` — **interactive**
Username is an argument; the password is prompted for through a callable
(`controller.login(username, get_password, get_2fa)`). So stdin alone is likely not enough —
a **PTY** is required to answer the password (and 2FA) prompts.
Wrapper options: `portable-pty` or `pty-process` in Rust.

**Measured 2026-10-01**, driven through a PTY with a throwaway username (no password sent):

```
Password:          <- written with no trailing newline, then the CLI blocks
```

Two properties of that prompt, both of which broke this wrapper before they were written down:

- **The prompt is not a line.** There is no `\n`, so a reader that only forwards complete lines
  forwards nothing at all: the engine never sees the prompt, never writes the password, and the
  window sits at "работаю" forever — the user can neither get in nor get out. Anything the child
  leaves unterminated for ~200 ms is treated as a prompt (`runner::PROMPT_IDLE`).
- **The CLI turns echo off itself.** Writing a known string and reading the master for five
  seconds produces no echo of it, so the password does not reach the transcript. That is the CLI
  holding up its end; our end is never writing the secret anywhere but the PTY.

The failure path is a plain error line and a clean exit:

```
Error: Authentication failed. Please check your username and password and try again.
```

The **2FA prompt text is still uncaptured**: it needs real credentials, and the account this was
measured against does not have 2FA enabled.

### `protonvpn signout`
Logs out and clears local credentials.

### `protonvpn config list` — parseable
```
Current configuration
Setting                  Value
-----------------------  ------------
netshield                malware-only
kill-switch              off
port-forwarding          on
custom-dns               off
vpn-accelerator          on
moderate-nat             off
ipv6                     on
anonymous-crash-reports  on
```

### `protonvpn config set <setting> <value>`
Settings: `netshield`, `kill-switch`, `port-forwarding`, `custom-dns`, `vpn-accelerator`,
`moderate-nat`, `ipv6`, `anonymous-crash-reports`.
`kill-switch` accepts exactly `{off|standard}` — there is **no `permanent`** through the CLI,
even though the underlying enum has `PERMANENT=2`.

---

## 2. Coexistence rules — important

The CLI refuses to run while the **official GTK app** is running:

```python
GTK_APP_ID = "proton.vpn.app.gtk"
# if GTK_APP_ID is on the session bus and this isn't a help request:
"Error: Proton VPN desktop app is currently running
 The CLI and GUI cannot run simultaneously. Please close the GUI application and try again."
```

Two implications for us:

1. Our GUI **must not own the bus name `proton.vpn.app.gtk`**, or it will break the CLI it
   depends on. Use our own app id.
2. If the official app is running, every CLI call we make fails with that message. We must
   detect it and show it prominently in the log pane rather than looking broken.
   (There is an `--allow-gui-concurrency` style flag in the CLI context object; worth checking
   whether it is reachable from the public flags before relying on it.)

---

## 3. What the wrapper can and cannot offer

| Feature | Available via CLI? |
|---|---|
| Connect / disconnect | yes |
| Connect by country / city | yes |
| Presets: P2P, Secure Core, Tor, random, fastest | yes |
| Specific server by ID (`IT#23`) | yes, as `SERVER_NAME` |
| Country + city lists with feature tags | yes (`countries list`, `cities list`) |
| Current status | yes, polled, human text |
| Account info | yes (`info`) |
| Settings read/write | yes (`config list` / `config set`) |
| Login / logout | yes, but interactive → needs a PTY |
| Per-server load / latency / full server browser | **no** |
| Live push state changes | **no** — polling only |
| NetShield filtering | yes — server-side, unaffected by CLI exit |
| Port forwarding | port is assigned and readable, **but the lease needs an external renewal script** |
| Kill switch beyond `off`/`standard` | **no** (`permanent` exists in the enum, not exposed) |

---

## 4. Phase 0 results

All measured on this machine, 2026-09-30. Raw outputs are saved as fixtures in
`crates/protonvpn-core/tests/fixtures/`.

### 4.1 Timing budget

| Invocation | Wall clock |
|---|---|
| `status` (disconnected) | 1.12 / 1.12 / 1.09 s |
| `status` (connected) | 0.94 s |
| `status` (after disconnect) | ~1.0 s |
| `connect --country CH` | 3.55 s |
| `connect --country NL` (switching) | 2.37 s |
| `disconnect` | 1.30 s |

**A status poll costs ~1 s** because it spawns a Python interpreter each time. Consequences for
the UI: never poll on a fixed 1–2 s timer; poll only while a transition is pending, then back off
to a slow idle interval (10 s+) and pause entirely when neither window nor tray is being
interacted with. This is the single biggest design constraint the CLI imposes.

### 4.2 Connected `status` output — the parser target

```
Status: Connected
Server: CH#274 in Zurich, Switzerland
Load: 44%
Protocol: wireguard
```

Disconnected:
```
Status: Disconnected
```
Both are `Key: Value` lines, exit code 0. Trivially parseable, no ANSI when not a TTY
(verify ANSI behaviour when a PTY is attached).

### 4.3 `connect` output

```
Connected to CH#274 in Zurich, Switzerland. 
Your new IP address is 149.88.27.213.

Note: Port forwarding is enabled but this server does not support it.
Connect to a P2P server to use port forwarding:protonvpn connect --p2p
```

Second line is optional; capability notes follow. **Do not trust `Your new IP address` as the
current egress IP** — for CH#274 it printed `149.88.27.213` while live egress was
`149.22.89.89`. Treat it as informational only.

### 4.4 Exit codes

| Case | Exit | Output |
|---|---|---|
| success (connect / disconnect / status) | `0` | see above |
| `disconnect` while already disconnected | `0` | `Disconnected.` — **idempotent**, safe to call blindly |
| `connect` already connected | `0` | **switches servers silently**; CH#274 → NL#450 in 2.37 s |
| invalid country (`--country ZZ`) | `2` | `Error: Invalid country code 'ZZ'. …` |
| invalid server (`ZZ#99`) | `2` | `Error: Invalid server ID 'ZZ#99'. …` |

So: `0` = success, `2` = validation/usage error. Connection-failure codes are still unknown;
the UI must fall back to "non-zero = show the raw output" rather than mapping every code.

### 4.5 NetworkManager artifacts

```
NAME                  TYPE       DEVICE           STATE
ProtonVPN CH#274      wireguard  proton0          activated
pvpn-killswitch-ipv6  dummy      ipv6leakintrf0   activated
```

- Profile naming convention: **`ProtonVPN <SERVER_NAME>`** — usable for correlation.
- `pvpn-killswitch-ipv6` (dummy iface `ipv6leakintrf0`) is active **even with
  `kill-switch off` in settings**. IPv6 egress is blocked (`curl -6` fails) while IPv4 egress
  goes through the VPN. So "kill switch off" does not mean "no protection".
- After `disconnect` **both profiles are removed** and the original public IP returns. No
  leftovers, no leak.

### 4.6 Still to capture

1. Connection-failure exit codes (requires a server that fails — hard to trigger on demand).
2. The `signin` **2FA** prompt (the password prompt and the logged-out `info` output were
   captured on 2026-10-01 — see §1 — and are no longer outstanding).
3. The exact CLI error when the official GTK app is running (needs the GTK app installed —
   it is not).
4. A **conclusive** NetShield test — see §4.7; the one attempted was inconclusive.

### 4.8 PTY capture results — the corpus is frozen

All fixtures were re-captured through a real PTY (`docs/architecture.md` §10.3) by
`scripts/capture-fixtures.sh`, into `crates/protonvpn-core/tests/fixtures/pty/` — 13 invocations,
each with a `.txt` of raw output and a `.meta.json` of argv, geometry, exit code and duration.
The original pipe captures are kept alongside in `tests/fixtures/pipe/` for reference.

Four results, and three of them are good news:

**1. The CLI emits no ANSI escape sequences at all.** Every capture reported `ansi=0` — no
colour, no cursor movement, not even for the tables, and not even on a TTY. `strip_ansi` stays in
the PTY module regardless: it is cheap, it normalises the `\r\n` the terminal layer introduces,
and `signin`-adjacent output is the most likely place for escapes to appear later.

**2. PTY output is identical to pipe output.** Diffed command by command: `status`, `info`,
`config list`, `cities list` and `countries list` matched exactly after CRLF normalisation. The
one apparent difference was a cache artifact — the PTY run refreshed the server list, so the pipe
run that followed did not print `Server list is outdated, updating...`. So the earlier decision to
re-capture was still correct as due diligence, but it turns out the pipe fixtures were valid.

**3. Terminal width does not affect the tables.** `countries list` captured at 120 and at 80
columns is byte-identical; the widest line is 40 characters and nothing wraps. Parsers must not
depend on width. `countries_list_cols80.txt` is kept as a regression guard.

**4. Everything is `\r\n`-terminated.** The PTY layer converts the CLI's `\n`. Any consumer that
reads raw bytes must normalise.

The one line parsers must learn to skip is the cache warm-up notice, which appears only when the
server list is stale:

```
Server list is outdated, updating... This may take a moment.
```

### 4.9 Ground-truth probe: geolocation is not trustworthy

Tested while connected to `NL#662`, querying three keyless JSON endpoints. All worked:

| Endpoint | Verdict |
|---|---|
| `https://ifconfig.co/json` | richest — `ip`, `country_iso`, `asn`, `asn_org` |
| `https://ipinfo.io/json` | `ip`, `city`, `region`, `country`, `org`, `loc`, `timezone` |
| `https://ifconfig.me/all.json` | `ip_addr`, `user_agent`, `port`, `method` |

**The important finding is a negative one.** The same IPv4 address, `205.147.16.120`, was
reported as:

- country **NL** by `ipinfo.io` (`org: AS208172 Proton AG`)
- country **US** by `ifconfig.co` (`asn: AS60068 Datacamp Limited`)

Both are Proton's infrastructure. The GeoIP databases simply disagree.

**Consequence: a geo consistency check would produce false alarms and must not be used as a
correctness signal.** The earlier idea of comparing "the CLI says Zurich" against "the probe says
another country" is dropped.

What *is* reliable, and what the probe is actually for:

1. **Did the egress IP change from the pre-connection baseline?** This is the real test of
   whether traffic is flowing through the tunnel. If the CLI says `Connected` and the probe
   returns the baseline address, the tunnel is not carrying traffic — a genuinely useful check
   that no CLI output can provide.
2. **Both address families.** Forced IPv4 and forced IPv6 were queried separately. IPv6 egress
   worked on this server (`2a02:6ea0:c041:6652::34`, a Proton/Datacamp prefix), and differed from
   the pre-VPN baseline (`2001:bb6:582:5558:...`) — so no leak. Note IPv6 support varies by
   server: on an earlier connection to `NL#450` an IPv6 request failed outright.
3. **Compare like with like.** Baseline and post-connection readings must come from the *same*
   endpoint, so a database disagreement can never masquerade as a state change.

Country and city from the probe are still worth *displaying* — they are simply not evidence.

> What the application does with all of this is in [`architecture.md`](architecture.md) §8, and it
> is **less** than the list above suggests: the probe keeps one reading and the comparison in item 1
> is not computed. It answered "no change" about a working tunnel whenever the application started
> while the VPN was already up, and it was the reason the SOCKS5 gate stayed shut. The finding here
> — that the comparison is the only meaningful use of an IP echo — stands; what changed is that the
> application leaves the judging to the person reading the card.

### 4.7 NetShield vs port forwarding — correcting an earlier claim

These two behave completely differently after the CLI exits, and an earlier draft wrongly
lumped them together.

**NetShield — works, nothing to warn about.**
The CLI's own help states it "blocks malicious and advertising domains **at the DNS level**".
The VPN connection's resolver is the gateway's `10.2.0.1`:

```
proton0   Current DNS Server: 10.2.0.1   (+DefaultRoute)
```

Filtering therefore happens at the gateway, for the lifetime of the connection, and does not
depend on any process of ours staying alive. The UI should present it as an ordinary working
setting.

*Caveat on the test:* querying `malware.wicar.org`, `malware.testcategory.com` and
`ads.testcategory.com` against `10.2.0.1` returned normal A records. That is **inconclusive**,
not a failure — those third-party test domains may simply not be in Proton's blocklist, and the
active mode was `malware-only`, not `malware-ads-trackers`. A conclusive test needs a domain
known to be blocked. Not worth blocking the GUI on.

**Port forwarding — available, but the lease needs an external script.**
Verified live on a P2P-capable server:

```
connect output        "Port forwarding is active on this server.
                       To get your forwarded port, run the natpmpc setup script."
/run/user/1000/Proton/VPN/forwarded_port     62393
ss -tnp | grep 10.2.0.1                      <nothing>   <- Local Agent is gone
forwarded_port over 40 s                     unchanged, mtime frozen at connect time
```

So the port **is** assigned and **is** readable from disk, but nothing renews it. The CLI says
this outright in `config set port-forwarding --help`:

> "The port assignment requires an external script to maintain the lease and retrieve the port
> number. **Without the script, the assigned port expires.**"

`natpmpc` is **not installed** on this machine.

**Consequence for the GUI — technically we do not need `natpmpc` at all.**

> [!IMPORTANT]
> **This is a technical finding, not a decision.** Whether we are *allowed* to use it is an open
> architectural question: direct NAT-PMP violates the project's "the only program we execute is
> `protonvpn`" rule ([`architecture.md`](architecture.md) §0, ruling in §10.1). Until that is
> settled, port forwarding is blocked. The evidence below is kept because it is what the ruling
> has to be made against, and because it removes `natpmpc` from the conversation either way.

An earlier plan was to supervise the `natpmpc` external script. That was tested and is
**unnecessary**: the gateway speaks plain NAT-PMP (RFC 6886) on `10.2.0.1:5351`, so we can
speak it ourselves. Verified live, from a bare UDP socket, no helper installed:

```
public-address request -> result=0  external_ip=46.29.25.99        (39 ms)
map udp                -> result=0  internal=39949  EXTERNAL=39949  lifetime=60s  (42 ms)
map tcp                -> result=0  internal=39949  EXTERNAL=39949  lifetime=60s  (40 ms)

renew after 5 s        -> same port 39949          (stable across renewals)
delete (lifetime=0)    -> result=0  external=0  lifetime=0          (released)
```

The port `39949` returned by NAT-PMP **matched exactly** what the CLI's own Local Agent had
written to `forwarded_port`, confirming both paths use the same mechanism.

Protocol shape, for the implementer:

```
request  (12 bytes): version=0 | op(1=udp,2=tcp) | reserved=0 | internal_port | suggested_ext | lifetime
response (16 bytes): version   | op             | result      | epoch | internal_port | ext_port | lifetime
```

`result=0` means success. The gateway returns **the same external port for TCP and UDP**, equal
to the internal port, and the mapping is **sticky within a VPN session** — repeated requests
return the same number rather than a fresh random one.

Consequences, all of them improvements:

1. **No `natpmpc` dependency** — it is not installed on this machine and no longer needs to be.
2. **No child process to supervise.** Renewal is a timer plus a 12-byte UDP datagram in our own
   process. The "stray process left behind" risk disappears entirely.
3. **Authoritative data.** We get the port, the lifetime and a result code directly, instead of
   scraping `natpmpc`'s stdout.
4. Renewing every **40 s** against a **60 s** lease leaves ample margin.

Calling this "writing a competing library" would be a misread: NAT-PMP is a generic IETF
protocol, not Proton's stack. Proton's own documentation points at `natpmpc`, which speaks
exactly this. The alternative — spawning `natpmpc` and parsing its output — remains available if
we would rather not own those ~40 lines.

#### Why the lease behaves this way

Port forwarding exists so that peers can open connections **to** us through the VPN exit.
Behind the VPN we can only make outbound connections; the exit servers have no way to know which
of their users an inbound packet is for, so they cannot forward anything unless a mapping is
explicitly asked for. The main use is BitTorrent (hence P2P-only servers): without a mapping you
can still download by connecting out, but you cannot accept inbound peers, cannot seed properly,
cannot reach NATed peers, and private trackers that require being connectable will not work.
Secondary uses are hosting anything reachable from outside while the machine is behind the VPN —
game servers, a web server, SSH.

Proton's own rules, which make this a process rather than a setting:

| Property | Value |
|---|---|
| Availability | P2P-optimised servers only, paid plans |
| Port assignment | **random**, not per-user, "by design" — a new request gives a new number |
| Lease lifetime | **60 seconds** |
| Refresh cadence | ~**every 45 s**, indefinitely |
| Mechanism | **NAT-PMP** — `natpmpc -a 1 0 udp 60 -g 10.2.0.1` and the same for `tcp` |
| Gateway | `10.2.0.1` — the same address as the VPN resolver |

Corroborating detail: the Local Agent binary contains `10.2.0.1:5351`, and **5351 is the standard
NAT-PMP port**. The official GUI therefore does exactly what `natpmpc` does, over the same
protocol, just from inside its own process. Driving `natpmpc` is using the sanctioned mechanism,
not a workaround.

#### The correctness rule this implies

> Either we supervise the lease and show the port, or we **do not show the port at all**.

`forwarded_port` is written once at connect time and becomes meaningless ~60 s later. Displaying
it without renewal is misinformation. Since `port-forwarding` is currently `on` in the user's
settings, this situation arises on every P2P server connection, not hypothetically.

---

## 5. Live session evidence

Sanitised transcript of the Phase 0 run, for the record.

```
baseline egress                      2001:bb6:582:5558:...
protonvpn status                     Status: Disconnected          (exit 0, 1.12 s)
protonvpn connect --country CH       Connected to CH#274 in Zurich, Switzerland.
                                     Your new IP address is 149.88.27.213.   (exit 0, 3.55 s)

  -> CLI process gone; only the pre-existing root split-tunnelling daemon remains

protonvpn status                     Status: Connected
                                     Server: CH#274 in Zurich, Switzerland
                                     Load: 44%
                                     Protocol: wireguard                (exit 0, 0.94 s)
egress (through tunnel)              149.22.89.89

protonvpn connect --country NL       Connected to NL#450 in Amsterdam, Netherlands.  (exit 0, 2.37 s)
protonvpn status                     Status: Connected / Server: NL#450 / 49% / wireguard

protonvpn disconnect                 Disconnected.                      (exit 0, 1.30 s)
protonvpn status                     Status: Disconnected
NM proton profiles                   none (both removed)
egress                              2001:bb6:582:5558:...  (restored)
```

**Verdict: the wrapper model works.** The tunnel is NetworkManager's, not the CLI's, so it
outlives the process; only Local-Agent-backed features are lost.
