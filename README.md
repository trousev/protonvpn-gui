# Proton VPN GUI (Linux) — a console-first wrapper around the official CLI

A GUI for Proton VPN on Linux that **wraps `protonvpn`** instead of reimplementing it, and shows
you every command it runs and every byte the CLI prints. The console is the product; everything
else is a view onto it or a reducer over it.

![The window, collapsed console bar at the bottom](images/window.png)

It is a wrapper and it does not pretend otherwise.

## Download

Releases are published on demand, not on every merge, and the newest one is always on the
[releases page](https://github.com/trousev/protonvpn-gui/releases/latest). Two assets are attached
to each release:

**AppImage** — nothing to install:

```sh
chmod +x ProtonVPN-GUI-<version>-x86_64.AppImage
./ProtonVPN-GUI-<version>-x86_64.AppImage
```

The image carries its version in its name and in its bytes (`--version` prints it), which is what
lets it **update itself**: it compares that number against the latest release, and can fetch and
verify a newer image and rename it over itself. Nothing downloaded is ever executed — the new image
takes effect the next time you start it — and the file it replaced is kept beside it as
`<name>.old` until that start proves the new one works. See [Updates](#updates).

**Tarball** — the same binary, plus the `.desktop` entry, icon, README and LICENSE:

```sh
tar -xzf protonvpn-gui-<version>-x86_64-linux.tar.gz
sudo install -m755 protonvpn-gui-<version>-x86_64-linux/protonvpn-gui /usr/local/bin/
```

Neither archive contains the `protonvpn` CLI: it is a system dependency (see Requirements), and
the AppImage calls it from `PATH` like the installed binary does.

On the first start the app writes its own menu entry — `~/.local/share/applications/protonvpn-gui.desktop`
— and its icon into the hicolor theme. That is not decoration: Wayland has no window icons at all,
and a desktop learns a window's name and icon only from a `.desktop` file whose name matches the
window's app id. Without it GNOME shows the window as an unknown application under a generic
gear. It is a setting (`Entry in the application menu`, on by default), and turning it off removes
both files.

Downloads can be verified rather than trusted — `SHA256SUMS` covers both assets:

```sh
sha256sum -c SHA256SUMS
gh attestation verify <the-asset> --repo trousev/protonvpn-gui
```

The attestation is signed proof that the artifact came out of this repository's release workflow,
which is the answer to "could someone have swapped it?" — see [`SECURITY.md`](SECURITY.md).

Versions are `X.Y.N`: `X.Y` is the latest release tag in the repository, `N` is the number of
commits in `main`. Nothing is bumped by hand — the number is simply where `main` stood when the
release was asked for.

## What it does

- Connect / disconnect: fastest server, a country, a city, a specific server (`IT#23`), and the
  `--p2p`, `--securecore`, `--tor`, `--random` presets.
- Country and city browsing, from `countries list` / `cities list` (features included).
- CLI settings read and written through `config list` / `config set`, with the values the CLI
  documents.
- Login and logout, including the password and 2FA prompts, which are answered over a PTY.
- **Port forwarding**, which the official CLI cannot do on its own: the lease is renewed from our
  own NAT-PMP client, and the port is shown with a copy button.
- **A local SOCKS5 proxy for the paranoid case**, also off by default: point an application at
  `127.0.0.1:1080` and it gets the system's internet while the tunnel is up — and a refusal, not a
  leak, the moment the tunnel is gone. See [The SOCKS5 door](#the-socks5-door).
- A tray icon that works with no window at all, and an autostart entry.

## The rules it follows

Two rules shape the whole design, and they are the reason the code looks the way it does:

1. **The only program this application executes is `protonvpn`.** It does not know how the
   connection works: no NetworkManager, no D-Bus, no keyring, no Proton-internal files. If the CLI
   says it is connected, it is connected. State comes from `protonvpn status`, never from
   inspecting the system.
2. **The console is the product.** Every invocation is recorded verbatim — argv, exit code,
   duration, raw output — and shown in a collapsible pane. Both the state interpreter and the
   console read that one stream, so they can never disagree about what the CLI said.

Three exceptions are sanctioned and bounded, each because the CLI genuinely cannot do the job:

| Exception | Why | Bounds |
|---|---|---|
| `curl` to an IP-echo service | the CLI's own `Your new IP address is …` is not the egress address (measured: it printed `149.88.27.213` while real egress was `149.22.89.89`) | read-only, third party, keyless |
| NAT-PMP to `10.2.0.1:5351` | the port-forwarding lease; the CLI only sets a preference and tells you to run a script | IANA-standard port, publicly documented gateway, opcode-0 probe first, degrades honestly |
| a local SOCKS5 listener, plus the kernel's own routing answer behind it | an application that must never reach the network without the VPN has nothing to hold on to otherwise | **off by default**, loopback (`127.0.0.0/8`) only, IPv4 + `CONNECT` only, fails closed |

An earlier exception was built and withdrawn: pushing the forwarded port into a local qBittorrent over its Web
API. It never worked against a real client, and convenience for one torrent client is not worth a
standing hole in the rule above — the port is displayed and copyable instead.

## The SOCKS5 door

The paranoid case, off by default in **Settings → Proxy**: an application that must never reach
the network without the VPN gets pointed at `127.0.0.1:1080` in its own settings, and this
application makes sure that door is either open onto the tunnel or shut.

- **While the tunnel is up**, the proxy relays. Domain names are resolved at the proxy and only
  after the gate says yes, so the application's own configuration never leaks the name; the lookup
  itself is the machine's, through the system resolver, exactly as it would be without a proxy.
- **The moment it is not**, every new connection is refused with a SOCKS5 "not allowed by
  ruleset", and every connection already being relayed is dropped.
- **The check is the kernel's own answer**, not a `status` poll: "which source address would an
  off-link packet use?" — four syscalls, no packets, no third party. A new connection is refused
  immediately if the answer moved, and what is already relaying is dropped within about a quarter
  of a second.
- **Every 30 seconds** (configurable, `0` turns it off) the same `curl` egress check the Overview
  page uses confirms that traffic is really going somewhere else. If the egress address is the
  pre-connection one again, the door shuts, whatever `protonvpn` says. This one needs the egress
  probe itself to be on (Settings → Polling); with it off, only the local route check remains.

Two things to know before turning it on, both deliberate:

- **It arms on evidence.** The gate opens only when the kernel's route differs from a route
  observed while the CLI said the tunnel was down. If you enable the proxy while the VPN is
  already connected, the app has never seen the other route, so the proxy stays shut and says so
  — reconnect once (Disconnect, then Connect) and it arms. The same holds after the door shuts
  itself: a gate closed on suspicion never reopens on a different route alone.
- **It is IPv4, `CONNECT` and loopback only, with no authentication.** An IPv6 literal is refused
  rather than guessed at; there is no `BIND` or `UDP ASSOCIATE`; and the listener cannot leave the
  loopback range, whatever the config file says. That is the same promise `ssh -D` makes — which
  also means any local process can use the door while it is open.
- **Local-network destinations are refused** while the door is open: everything but a loopback
  address must come from the tunnel's own source address, so a router or a NAS is not reachable
  through the proxy. It relays to the internet through the tunnel, not onto your LAN.

## Requirements

- Linux with a Wayland or X11 session.
- The official CLI: `protonvpn` (the `proton-vpn-cli` / `protonvpn` apt package), signed in.
  **This is a runtime dependency, not bundled.**
- For the tray: a StatusNotifierItem host. GNOME needs the AppIndicator extension; without a tray
  the app detects that and refuses to hide the window into nothing.
- `curl` for the optional egress check (`probe`), which is on by default and can be switched off.

The app must not own the bus name `proton.vpn.app.gtk`, and does not: it uses its own app id, so
the CLI keeps working while the GUI runs.

## Build and run

```sh
cargo build --release -p protonvpn-gui
./target/release/protonvpn-gui
```

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` will fetch it.

`protonvpn-core` has no UI dependency and can be built and tested on its own:

```sh
cargo test                                  # unit tests + fixture-corpus guarantees
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

## Updates

An AppImage is a file, and nothing upgrades it: no package manager owns it, and the file that would
have to be replaced is the one currently running. So the application does it, within bounds
([`docs/architecture.md`](docs/architecture.md) §14):

- it asks **its own release page** for `SHA256SUMS` — one URL, no GitHub API, no rate limit, and no
  third party: `https://github.com/trousev/protonvpn-gui/releases/latest/download/SHA256SUMS`;
- the version is read from the asset's own name, so the version and the checksum arrive together;
- the image is downloaded, checked against that checksum and against the type-2 AppImage marker,
  and **renamed** over the running one; it is never executed by the update;
- the previous image stays as `<name>.old` for exactly one start, then goes.

Four policies, in **Settings → General → Application · Updates**, and `download` is the default:

| Policy | What happens without being asked |
|---|---|
| `do not check` | nothing; the release page is only contacted if you press the button |
| `notify only` | the latest release is reported, nothing is downloaded |
| `download` | the image is downloaded and verified, and waits next to the running one |
| `download and install` | …and renamed into place; it takes effect at the next start |

Every step stays available as a button whatever the policy says. The check runs a few seconds after
start and then at most once a day (`last_check` is remembered in the config, so restarting the
application does not mean checking again), and it never blocks anything: the check and the download
live on their own threads, and cancelling one kills `curl` and deletes the partial file.

**What the checksum is worth.** It catches a truncated download, a proxy, a mirror serving an older
file. It is **not** proof of authorship — whoever can answer for `github.com` can serve the image
and the checksum together. That is what the release's build-provenance attestation is for, and it
is checked by hand, not by the application:

```sh
gh attestation verify ProtonVPN-GUI-<version>-x86_64.AppImage --repo trousev/protonvpn-gui
```

Where the image cannot be written — root-owned, in `/opt`, a read-only filesystem — the app says so
and leaves it alone: it never runs `sudo`. A build that is not an AppImage (the tarball, a `cargo
run`) compares nothing, reports what the release page says and offers nothing.

## Configuration

Everything the app itself owns lives in one file:

```
~/.config/protonvpn-gui/config.json
```

It holds the app's own preferences (connect on start, start minimized, autostart, the app-menu
entry, the egress probe, the port-forwarding lease, the SOCKS5 address, port and verification
interval, and the update policy with the date of the last check) and the connections you build. There are no
secrets in it:

- **Proton's own `settings.json` and `app-config.json` are never read or written.** Those belong
  to the official app; the CLI settings are read through `protonvpn config list` like any other
  state.
- **Nothing the app needs in confidence is stored.** A password or a 2FA code goes straight to the
  CLI's PTY and lives in memory only while that command runs.

Autostart is a plain `~/.config/autostart/protonvpn-gui.desktop`, written only when you ask for it.
The application entry and its icon live in `~/.local/share/{applications,icons/hicolor}` and are
kept in step with the setting described above; [`docs/architecture.md`](docs/architecture.md) §12
is the contract for all three files.

## Language

The interface is English and Russian, and it follows the desktop unless you pick one in
**Settings → General**. English is the *source* language: every sentence the application says to a
person is a message in `crates/protonvpn-core/i18n/en/`, and `ru/` beside it is a translation of
exactly that set.

The catalogue is [Project Fluent](https://projectfluent.org/), and it is checked **before the
crate compiles**: a language that is missing a message, a file or a `$variable` does not build.
That is deliberate. A fallback to English at run time is the kind of gap that goes unnoticed for a
year, so it is a compile error instead — which also means an untranslated string can never
disappear into a release.

Adding a language is adding a directory:

```sh
mkdir crates/protonvpn-core/i18n/de
./scripts/translate     # reads $OPENAI_API_KEY, fills in what is missing
```

`scripts/translate` sends only the messages a language is actually missing, along with the
developer comment above each one — where it appears and how much room it has — and finishes by
running the real check. `./scripts/translate --check` reports gaps without calling anything.

What is **not** translated is data: a server name, a country, a city, an exit code, the command
line, the CLI's own output. The console shows those exactly as they arrived, which is the whole
reason it is worth reading. [`docs/i18n.md`](docs/i18n.md) is the full contract.

## Contributing

`main` is protected. Every change goes through a pull request and cannot be merged until CI is
green — the same gates you can run yourself:

```sh
cargo fmt --all --check
./scripts/check-linux-deps.sh
./scripts/translate --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

**Before you open a pull request, run `./scripts/translate`** if you touched a string: it fills in
the translations a new or changed message needs, and it fails loudly rather than leaving a language
half-done.

No review is required, and no direct pushes are possible, including for the maintainer: if CI is
red, `main` cannot move. See [`SECURITY.md`](SECURITY.md) for the workflow hardening this repository
applies, and `.github/dependabot.yml` for how the pinned dependencies get updated.

## Packaging

AppImage first — Flatpak was dropped deliberately, because the sandbox would fight both the host
`protonvpn` CLI and tray-name ownership for no benefit a wrapper needs. See
[`packaging/appimage/build.sh`](packaging/appimage/build.sh).

The release job builds the AppImage in a job that can only **read**, then publishes it from the job
that may write. The toolchain — `linuxdeploy` and the output plugin that carries `appimagetool` —
and the AppImage runtime are pinned by version and checked against a SHA-256 before they run, so a
build neither trusts a moving `continuous` tag nor executes unverified bytes. The third-party
toolchain never runs in the job that holds `contents: write`.

The tarball is still attached as the plain fallback, and `packaging/appimage/build.sh` is the whole
AppImage path on its own.

Releases are on demand: `./scripts/release.sh` dispatches the release workflow with `gh`, and the
workflow is the only thing that publishes. Nothing goes out because a pull request was merged —
code landing and a build being handed to the world are two decisions, and only the second one needs
someone to mean it.

## Honest limitations

- **State is polled.** `protonvpn status` costs about a second (it starts a Python interpreter),
  so the idle cadence is five minutes, with fresh reads right after any command that could have
  moved the tunnel, and when you open the window or click the tray. Every piece of state is shown
  with its age (`updated 3 mins ago`) — never with the word "stale", because the number is the
  message.
- **`protonvpn connect` exits.** The tunnel and its NetworkManager-resident protections survive
  (that is NetworkManager's business, not ours); only the in-process Local Agent does not, which is
  exactly why the port lease is ours to maintain.
- **No per-server browser.** `protonvpn servers` only prints a web link, so load and latency per
  server are not available. Country and city granularity is.
- **GeoIP from the probe is display-only.** The same address was reported as NL by `ipinfo.io` and
  US by `ifconfig.co`; only "did the egress address change from the baseline?" means anything.
- **Closing the window destroys it; the tray recreates it.** winit cannot hide a window on
  Wayland at all (`set_visible` is a no-op there), so "close to tray" is implemented by destroying
  the window and opening a new one from the tray. That is also why the app is an `iced::daemon`
  and not an `iced::application`: an application exits when its last window is destroyed.
- **The SOCKS5 proxy is a door, not a firewall.** It relays one application's traffic while the
  tunnel is proven and refuses when it is not; it does not stop the application from ignoring its
  proxy setting, and while the door is open any local process can use it. Its blind spot is the
  source address it pins: a route change that keeps the same source address is invisible to it,
  and between the route check and the connect sit the name lookup and the dial — up to ten
  seconds in which a route that moves can expose the destination and your real address, though no
  application byte. All of it is in `docs/architecture.md` §13.2. Turn on the CLI's own kill
  switch if you want the network itself to be unforgiving.
- **Desktop notifications are not implemented.** They would need either another program or the
  session bus beyond the tray, and neither is sanctioned yet. A human decision, not an oversight.
- **The window costs 195 crates, and that is where they all are.** The wrapper itself —
  `protonvpn-core`: PTY runner, interpreter, launcher, poller, probe, NAT-PMP — is 26.
  The rest is `iced` 0.14, and under it `winit`, `softbuffer` and the tray's `zbus`. `Cargo.lock`
  also lists Android, Windows and macOS crates, because that is what `winit` declares and a lock
  file is a union over every target; none of them is compiled here. This is a Linux-only tool, and
  `scripts/check-linux-deps.sh` fails the build if one of them would be, or if the graph grows past
  the number recorded in the script — adding a dependency should be an edit someone reviewed, not a
  side effect.
- **The AppImage inherits the build runner's glibc floor (currently 2.39).** It is assembled on
  `ubuntu-24.04`, the same image CI uses, so it will not start on an older distribution even though
  nothing else in the bundle would prevent it. Moving the AppImage job — not the publish job — to
  an older runner is the change that fixes that, and it is deliberate when it happens.

## Layout

```
crates/protonvpn-core/     all VPN logic, no UI dependency (the tray must work headless)
  runner.rs                spawn `protonvpn …` on a PTY, stream lines, single-flight queue
  logbus.rs                one ordered verbatim stream, two consumers
  interpreter.rs           pure reducer: (AppState, LogEvent) -> AppState
  launcher.rs              intent -> argv, and nothing else
  parse.rs                 parsers, written against the frozen PTY fixture corpus
  poll.rs                  five-minute idle cadence, immediate after a change, attention-driven
  probe.rs                 the `curl` ground-truth probe (exception #1)
  net/natpmp.rs            the port-forwarding lease (exception #2)
  socks5.rs                the local SOCKS5 proxy (exception #3)
  net/route.rs             the kernel's routing answer the proxy's gate reads
  engine.rs                the one thread that owns state
  i18n/                    every word the app says: one directory per language, English first
  build.rs                 compiles the catalogue; an incomplete translation does not build
crates/protonvpn-gui/      views only: window, console pane, tray, the .desktop entries
scripts/translate          fills in missing translations with an LLM; run it before a pull request
```

The design contract is [`docs/architecture.md`](docs/architecture.md); it wins over everything
else, including [`docs/plan.md`](docs/plan.md). The captured behaviour of `protonvpn` 1.0.3 is in
[`docs/cli-surface.md`](docs/cli-surface.md).

## License

BSD-2-Clause. It wraps GPL-3.0 software; it does not link against or copy it.
