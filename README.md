# Proton VPN GUI (Linux) — a console-first wrapper around the official CLI

A GUI for Proton VPN on Linux that **wraps `protonvpn`** instead of reimplementing it, and shows
you every command it runs and every byte the CLI prints. The console is the product; everything
else is a view onto it or a reducer over it.

![The window, collapsed console bar at the bottom](images/window.png)

It is a wrapper and it does not pretend otherwise.

## Download

Every merge to `main` publishes a release, so the newest build is always on the
[releases page](https://github.com/trousev/protonvpn-gui/releases/latest):

```sh
# the asset is attached to every release
tar -xzf protonvpn-gui-<version>-x86_64-linux.tar.gz
sudo install -m755 protonvpn-gui-<version>-x86_64-linux/protonvpn-gui /usr/local/bin/
```

The tarball contains the binary, the `.desktop` entry, the icon, the README and the LICENSE. The
`protonvpn` CLI itself is **not** included — it is a system dependency (see Requirements).

Downloads can be verified rather than trusted:

```sh
sha256sum -c SHA256SUMS
gh attestation verify protonvpn-gui-<version>-x86_64-linux.tar.gz --repo trousev/protonvpn-gui
```

The attestation is signed proof that the artifact came out of this repository's release workflow,
which is the answer to "could someone have swapped it?" — see [`SECURITY.md`](SECURITY.md).

Versions are `X.Y.N`: `X.Y` is the latest release tag in the repository, `N` is the number of
commits in `main`. Nothing is bumped by hand, and every merge gets a version.

## What it does

- Connect / disconnect: fastest server, a country, a city, a specific server (`IT#23`), and the
  `--p2p`, `--securecore`, `--tor`, `--random` presets.
- Country and city browsing, from `countries list` / `cities list` (features included).
- CLI settings read and written through `config list` / `config set`, with the values the CLI
  documents.
- Login and logout, including the password and 2FA prompts, which are answered over a PTY.
- **Port forwarding**, which the official CLI cannot do on its own: the lease is renewed from our
  own NAT-PMP client, and the port is shown with a copy button.
- An optional, off-by-default qBittorrent push so the forwarded port lands in your torrent client
  without a copy-paste.
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
| qBittorrent Web API | hand the forwarded port to the torrent client | **off by default**, localhost only |

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

## Configuration

Everything the app itself owns lives in one file:

```
~/.config/protonvpn-gui/config.json
```

It holds the app's own preferences (connect on start, start minimized, autostart, the egress
probe, the port-forwarding lease, and the qBittorrent host/port/user). Two deliberate omissions:

- **Proton's own `settings.json` and `app-config.json` are never read or written.** Those belong
  to the official app; the CLI settings are read through `protonvpn config list` like any other
  state.
- **The qBittorrent password is never written to disk.** It lives in memory for the session only.

Autostart is a plain `~/.config/autostart/protonvpn-gui.desktop`, written only when you ask for it.

## Contributing

`main` is protected. Every change goes through a pull request and cannot be merged until CI is
green — the same three gates you can run yourself:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

No review is required, and no direct pushes are possible, including for the maintainer: if CI is
red, `main` cannot move. See [`SECURITY.md`](SECURITY.md) for the workflow hardening this repository
applies, and `.github/dependabot.yml` for how the pinned dependencies get updated.

## Packaging

AppImage only — Flatpak was dropped deliberately, because the sandbox would fight both the host
`protonvpn` CLI and tray-name ownership for no benefit a wrapper needs. See
[`packaging/appimage/build.sh`](packaging/appimage/build.sh).

The release job ships a **tarball**, not an AppImage, and that is a deliberate trade: building an
AppImage means downloading `linuxdeploy` and `appimagetool` — third-party binaries, historically
referenced by a moving `continuous` tag — into a job that holds `contents: write`. For a project
whose rule is "one program, and we know exactly which", a tarball built by `scripts/release.sh` from
a pinned toolchain is the smaller risk. Run `packaging/appimage/build.sh` locally if you want the
AppImage.

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
- **Desktop notifications are not implemented.** They would need either another program or the
  session bus beyond the tray, and neither is sanctioned yet. A human decision, not an oversight.

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
  qbittorrent.rs           the opt-in local push (exception #3)
  engine.rs                the one thread that owns state
crates/protonvpn-gui/      views only: window, console pane, tray, autostart
```

The design contract is [`docs/architecture.md`](docs/architecture.md); it wins over everything
else, including [`docs/plan.md`](docs/plan.md). The captured behaviour of `protonvpn` 1.0.3 is in
[`docs/cli-surface.md`](docs/cli-surface.md).

## License

BSD-2-Clause. It wraps GPL-3.0 software; it does not link against or copy it.
