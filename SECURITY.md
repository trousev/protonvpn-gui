# Security

## Reporting a vulnerability

Use **[Report a vulnerability](https://github.com/trousev/protonvpn-gui/security/advisories/new)**
on the Security tab — a private advisory that only the maintainer sees. Please do not open a public
issue for anything that could be exploited before a fix exists.

There is no bug bounty, and no promise of a response time. There is a promise to read it.

## What this program is allowed to do

The project's one rule is that **the only program it executes is `protonvpn`** — the official
Proton VPN CLI. It does not read NetworkManager, D-Bus state, the keyring, or Proton's own config
files, and it does not take the session-bus name `proton.vpn.app.gtk`, because doing so would break
the CLI it depends on.

Exactly four exceptions exist, each bounded, and they are listed with their reasoning in
[`docs/architecture.md`](docs/architecture.md) §0:

| Exception | What it can touch |
|---|---|
| `curl` to an IP-echo service | a keyless third-party URL, to see whether the egress address actually changed |
| NAT-PMP to the documented Proton gateway `10.2.0.1:5351` | the port-forwarding lease, UDP only |
| a local SOCKS5 listener, and the kernel's routing answer behind it | loopback (`127.0.0.0/8`) only, **off by default**; the route read is a connected UDP socket that is never written to |
| `curl` to this project's own release page | two HTTPS URLs on `github.com` — the `SHA256SUMS` of the latest release and the AppImage it names — for the AppImage updater; the checksum is verified against that file, and **nothing downloaded is ever executed** (see [Updates](#updates)) |

An earlier exception — an opt-in push of the forwarded port into a local qBittorrent — was built and then
removed: it never worked against a real client, and it is not worth a permanent hole in the rule
above. See [`docs/architecture.md`](docs/architecture.md) §10.4.

A further exception is a decision for a human, not a patch.

The SOCKS5 proxy deserves its own line here, because it is the feature a reader will suspect:
it relays an application's traffic only while the kernel still routes it the way the tunnel did,
it re-reads that route every 200 ms, and it drops what it has already relayed the moment the
answer changes. What it cannot promise is written down in
[`docs/architecture.md`](docs/architecture.md) §13.2 rather than left to be discovered: a route
change that keeps the same source address, DNS through the system resolver, IPv6 refused rather
than guessed at, and the window between the route check and the connect — the name lookup plus up
to ten seconds of dial — in which a handshake can carry the destination and your real address,
though never a byte of the application's. It is also loopback-only with **no authentication**: any
local process can use the door while it is open, which is the same promise `ssh -D` makes.

## Updates

An AppImage cannot be updated by anything except itself, so this application does that — with two
hooks into the machine: it replaces one file (the image it was started from) and it writes its own
config. Both are in [`docs/architecture.md`](docs/architecture.md) §14; the parts that belong in a
security document are these.

**What the checksum proves.** The image is checked against `SHA256SUMS` fetched from the same
release, over the same kind of connection, from the same origin. That catches a truncated
download, a proxy that injected something, a mirror serving an older file, and a resumed transfer
that went wrong. **It is not proof of authorship** — whoever can answer for `github.com` can serve
both files, and the check would agree with itself. Authorship is the provenance attestation, which
is not verified by the application:

```sh
gh attestation verify ProtonVPN-GUI-<version>-x86_64.AppImage --repo trousev/protonvpn-gui
```

A signature verified against a key pinned in the binary is the thing that would close this without
a third party; it is not in this version, and it is not pretended to be.

**What is executed.** Nothing, ever, by the updater. The verified file is renamed over the old one
and takes effect at the next start; the running process keeps the image it was started from. There
is no `--appimage-extract-and-run`, no self-restart path, and no way for a downloaded file to run
except by the user starting it. The previous image is kept beside it as `<name>.old` until the next
start proves the new one works.

**Where it refuses to act.** An image that cannot be written to — root-owned, a read-only
filesystem — is reported and left alone: this program never runs `sudo` and has no privilege path.
A build that is not an AppImage replaces nothing and says so. The shape check (an ELF with the
type-2 AppImage marker) rejects a document that is not an image at all, which is what a captive
portal would serve.

**What it cannot do to your VPN.** Nothing. The updater has never heard of Proton: it does not
touch the CLI, its queue, the tunnel state, the keyring or any Proton file, and a failed or
cancelled update changes nothing about a connection.

## Secrets

- Passwords and 2FA codes are written to the CLI's terminal (a PTY) and **never** into the
  transcript, the log, or the config file.
- The application stores one file of its own, `~/.config/protonvpn-gui/config.json`, and it has no
  secret fields: the config format is a list of booleans, connection profiles and one preset name.

## Supply chain

Open source, so the interesting attacker is not the user but a pull request.

- **Workflows never use `pull_request_target`.** Pull requests run with `pull_request`: a fork's
  code gets a read-only token and no secrets. There is nothing else to steal.
- **`GITHUB_TOKEN` is read-only by default** — at the repository level and again in each workflow —
  and only the release job elevates, to `contents: write`, because it has to create a tag.
- **Every action is pinned to a full commit SHA**, never a tag, and only GitHub-owned actions are
  used at all. Dependabot proposes the bumps.
- **No dependency or build cache in CI.** A cache is writable by a pull request and readable by
  later builds; the minutes saved are not worth that class of risk.
- **Releases are built only from `main`**, only after the same formatting, clippy and test gate
  that protected the merge, and only when a maintainer asks for one — the release workflow has no
  `push` trigger, so no merge publishes by itself. The artifact gets a **build provenance
  attestation** so a download can be verified rather than trusted:

  ```sh
  sha256sum -c SHA256SUMS
  gh attestation verify protonvpn-gui-<version>-x86_64-linux.tar.gz --repo trousev/protonvpn-gui
  ```
- **Secret scanning and push protection are enabled** on the repository, so a token committed by
  accident is blocked rather than published.

Found a hole in the above? That is a vulnerability report, and the top of this file says how.
