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

Exactly three exceptions exist, each bounded, and they are listed with their reasoning in
[`docs/architecture.md`](docs/architecture.md) §0:

| Exception | What it can touch |
|---|---|
| `curl` to an IP-echo service | a keyless third-party URL, to see whether the egress address actually changed |
| NAT-PMP to the documented Proton gateway `10.2.0.1:5351` | the port-forwarding lease, UDP only |
| qBittorrent Web API | `localhost` only, **off by default** |

Anything that would become a fourth exception is a decision for a human, not a patch.

## Secrets

- Passwords and 2FA codes are written to the CLI's terminal (a PTY) and **never** into the
  transcript, the log, or the config file.
- The optional qBittorrent password is held in memory for the session only; it is not part of the
  config format, and a test asserts that it cannot be.
- The application stores one file of its own, `~/.config/protonvpn-gui/config.json`.

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
  that protected the merge, and the artifact gets a **build provenance attestation** so a download
  can be verified rather than trusted:

  ```sh
  sha256sum -c SHA256SUMS
  gh attestation verify protonvpn-gui-<version>-x86_64-linux.tar.gz --repo trousev/protonvpn-gui
  ```
- **Secret scanning and push protection are enabled** on the repository, so a token committed by
  accident is blocked rather than published.

Found a hole in the above? That is a vulnerability report, and the top of this file says how.
