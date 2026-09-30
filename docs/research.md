# ProtonVPN on Linux — research findings

Date: 2026-09-30
Author: research pass before writing any code
Scope: everything below was verified **on the actual machine** (`Ubuntu 26.04`, GNOME)
unless explicitly marked otherwise.

> **Status note (post-decision).** §2 and §3.4 below record the research conclusion, which was
> "write a native Rust client". That recommendation was **rejected** in favour of wrapping the
> official CLI. The findings in this document still stand and are what the wrapper design rests
> on — in particular §1 (no VPN daemon to attach to) and the Local Agent consequences, which are
> exactly why the wrapper has the limitations listed in [`plan.md`](plan.md) §2.
> See [`cli-surface.md`](cli-surface.md) for the contract actually being wrapped.

---

## 0. Environment inventory (verified)

| Item | Value |
|---|---|
| Distro | Ubuntu 26.04 LTS (Resolute Raccoon) |
| Desktop | `gnome-shell`, `XDG_SESSION_TYPE` empty in probe shell |
| Tray host | `org.kde.StatusNotifierWatcher` **present** on session bus |
| GNOME tray extension | `ubuntu-appindicators@ubuntu.com` installed |
| `libayatana-appindicator3-1` | installed (0.5.94) — but **no** `-dev` headers |
| NetworkManager | 1.54.3, `active`, system D-Bus reachable |
| WireGuard | kernel module present (`wireguard.ko.zst`) |
| Flatpak | 1.16.6, `flathub` remote, `org.freedesktop.Platform` 24.08 + 25.08, `org.kde.Platform` 6.10 |
| `org.gnome.Platform` runtime | **not** installed |
| Rust | cargo/rustc 1.97.1 |
| AppImage tooling | **not** installed (`appimagetool`/`linuxdeploy` absent); `fusermount3` present |
| Dev headers | `gtk4`, `libadwaita-1`, `glib-2.0`, `libsecret-1`, `dbus-1` — **all missing** |
| Vulkan | ICDs for intel/nvidia/radeon/lvp/nouveau present |

### Official Proton stack installed (apt, `protonvpn-stable-release`)

```
proton-vpn-cli                 1.0.3     /usr/bin/protonvpn  (Python entry script)
proton-vpn-daemon              0.13.8    runs as root, systemd
python3-proton-vpn-api-core    5.5.11
python3-proton-vpn-local-agent 1.6.3     local_agent.abi3.so (compiled Rust/pyo3)
python3-proton-core            0.7.4
python3-proton-keyring-linux   0.2.3
```

There is **no GTK app installed** — only leftovers: `~/.config/autostart/proton.vpn.app.gtk.desktop`
and `~/.config/Proton/VPN/*.json`. That desktop entry is the model for requirement #1.

User is logged in: `protonvpn info` → `Account: 'trousev'`.

---

## 1. Architecture of the official stack

The single most important finding:

> **There is no "VPN daemon". There is no local API to control the VPN.**

The only root daemon (`me.proton.vpn.split_tunneling.service`, `python3 -m proton.vpn.daemon`,
v0.13.8) does **only split tunnelling**. Its `__main__.py` calls `init_split_tunneling_daemon()`
and nothing else.

The actual VPN connection is orchestrated **inside the user's own long-lived process** by
`proton-vpn-api-core`. The official GTK app *is* that process. Consequences:

1. The GUI is not a client of anything — it is the engine's host.
2. Whichever process owns the connection also owns the Local Agent and therefore
   Netshield / port forwarding / kill-switch state.
3. If that process dies, NetworkManager keeps the tunnel up, but the Local Agent dies.

Verified: no Unix socket, no HTTP server, no gRPC listener anywhere in the stack.
(`aiohttp` appears only as an HTTP **client**.) `ss -xlp` shows nothing Proton-owned.
The only runtime artifacts are files: `/run/user/1000/Proton/VPN/forwarded_port` and
`/run/user/1000/Proton/proton-sso.lock`.

### 1.1 The one real local D-Bus API: split tunnelling

Live-introspected, not guessed:

```
bus:       system
name:      me.proton.vpn.split_tunneling
path:      /me/proton/vpn/split_tunneling
interface: me.proton.vpn.split_tunneling

SetConfig(uid: q, config: a{sv})
GetConfig(uid: q) -> a{sv}
ClearConfig(uid: q)
GetAllConfigs() -> a(qa{sv})
LogStatus()
```

Policy file `/etc/dbus-1/system.d/me.proton.vpn.split_tunneling.conf` allows any client to
talk to it. `uid` is the Unix uid; the config dict carries mode (`exclude`/`include`),
`app_paths`, `ip_ranges`.

**This is reusable for free** — our GUI can drive split tunnelling without reimplementing it.

### 1.2 How the tunnel is actually built

`.../backend/networkmanager/protocol/wireguard/wireguard.py`:

- Profile is a **NetworkManager WireGuard connection** built through the NM D-Bus API
  (`NM.WireGuardPeer.new()`, `peer.set_endpoint(...)`, `peer.append_allowed_ip("0.0.0.0/0")`,
  `peer.append_allowed_ip("::/0")`, `peer.set_public_key(server.x25519_pk)`).
- `wireguard_config.append_peer(peer)`, then the private key comes from
  `credentials.pubkey_credentials.wg_private_key`.
- NM connection name is user-owned and derived per server; profile is persisted by NM.
- Kill switch = **separate NM "kill switch" connection profiles**
  (`killswitch/default/` and `killswitch/wireguard/`). Values: `OFF=0`, `ON=1`, `PERMANENT=2`.

So: the tunnel is plain, standard WireGuard at the NM level. No Proton-specific kernel magic.

### 1.3 The Local Agent

`local_agent.abi3.so` is **Rust** (`local-agent-rs`, pyo3, tokio + rustls), GPL-3.0 upstream.

Strings extracted from the binary:

```
10.2.0.1:65432          <- gateway internal address for the agent
10.2.0.1:5351
src/agent_connector.rs, src/agent_connection.rs, src/listener.rs, src/port_forwarding.rs
rustls-0.23.12, tokio-1.39.2, pyo3-0.24.0
fields: version, operation, response_code, internal_port, external_port, lifetime_seconds,
        gateway_epoch_seconds, features, netshield_level, bouncing, randomized-nat,
        split-tcp, port-forwarding, forwarded-port
```

It is a **TLS client** to the gateway, authenticated with a **client certificate**, carrying a
bincode/JSON message protocol (`LocalAgentError`, `Status`, `AgentFeatures`). It is *not* gRPC
and *not* local — it dials the VPN gateway over the tunnel.

**Corrected, after live testing** (see [`cli-surface.md`](cli-surface.md) §4.7): the Local Agent
is *not* what enforces NetShield — that is done by the gateway resolver (`10.2.0.1`) and works
regardless of the client. The user-visible duty actually lost when the client process exits is
**port-forwarding lease renewal** (`forwarded_port` is written once and then frozen). The LA's
other roles — randomized NAT, split TCP, connection-details reporting, hard-jail state — are
either informational or not exposed by the CLI at all.

### 1.4 Credentials — the key finding

The whole session is in **libsecret**, readable by any process of the user:

```
schema:      org.freedesktop.Secret.Generic
application: "Python keyring library"
service:     "Proton"
username:    "proton-sso-account-<base32(account).lower().rstrip('=')>"
index key:   "proton-sso-accounts"   -> JSON list of account names
```

Verified for `trousev` → key `proton-sso-account-orzg65ltmv3a`, entry found, 2484 bytes.

Structure (keys only; values not inspected beyond type/length):

```
UID, AccessToken, RefreshToken, Scopes, Environment, AccountName, LastUseData,
vpn:
  UID
  certificate:
    SerialNumber            str  (11)
    ClientKeyFingerprint    str  (88)
    ClientKey               str  (112)  <- WireGuard private key (base64, 32 bytes)
    Certificate             str  (741)  <- PEM, Local Agent client cert
    ExpirationTime, RefreshTime, Mode, DeviceName,
    ServerPublicKeyMode, ServerPublicKey (112)
  secrets:
    ed25519_privatekey      str  (44)   <- Local Agent TLS client key
  location: IP, Country, ISP, Long, Lat
  account: ... MaxTier, MaxConnect, Groups, ...
```

**Everything needed to connect is already on disk, no login required.**

### 1.5 REST API surface (from the installed python sources)

```
/vpn/logicals            server list; Servers[].X25519PublicKey, EntryIP, ExitIP, domain
/vpn/v1/loads            server load
/vpn/v1/location         client geolocation
/vpn/v1/certificate      register ed25519 pubkey -> cert + WireGuard key
/vpn/v1/cities/names, /vpn/v1/nps/*, /vpn/v2/clientconfig, /vpn/v2/status/{token}/binary
/auth/...                SRP auth: /auth/info, /auth/2fa, /auth/refresh, /auth/v4/sessions/forks
```

`PhysicalServer.x25519_pk` = `data["X25519PublicKey"]`, base64. Combined with the WireGuard
private key from the keyring, this is a complete standard WireGuard peer configuration.

### 1.6 Config files

`~/.config/Proton/VPN/settings.json` — protocol, killswitch, custom_dns, ipv6, features
(netshield, moderate_nat, vpn_accelerator, port_forwarding, split_tunneling).

`~/.config/Proton/VPN/app-config.json` — **this is exactly requirement #1 and #2**:

```json
{ "tray_pinned_servers": [], "connect_at_app_startup": "CH", "start_app_minimized": true }
```

`~/.config/autostart/proton.vpn.app.gtk.desktop` — `Exec=protonvpn-app`, `Terminal=false`.

---

## 2. Answer to Q1: is there a stable documented local API? Should we wrap the CLI?

**No supported public local API exists.** What exists:

| Thing | Status | Verdict |
|---|---|---|
| `me.proton.vpn.split_tunneling` D-Bus | works, introspectable, stable-ish | **reuse it** |
| VPN control D-Bus / socket / gRPC | does not exist | — |
| `protonvpn` CLI 1.0.3 | one-shot, human-oriented | bad foundation |
| `protonvpn-cli` v3 (old) | dead/replaced | ignore |
| REST API + cert endpoint | unofficial but stable, used by every 3rd-party client | **this is the real API** |

Why wrapping the CLI is a trap:

- It is **one-shot**: it connects, prints, exits. No `--background`, no `--json`
  (`protonvpn connect --help` verified). `connect` only calls `disconnect()` on *failure*,
  so the NM tunnel survives the process — but the Local Agent does not.
- `protonvpn status` is human text (`Status: Disconnected`), no machine format.
- No live state: a tray app would have to poll and parse prose.
- Netshield / port forwarding / kill-switch state would silently rot once the CLI exits.
- It is GPL-3.0 and drags in a Python runtime.

**Conclusion:** the correct split is
*REST API + NetworkManager D-Bus + (later) Local Agent* for a native client,
and the existing split-tunnelling D-Bus service reused as-is.
A CLI wrapper is only acceptable as a throwaway spike.

---

## 3. Answer to Q2: what to write the GUI in

### 3.1 Current crate facts (crates.io, 2026-09-30)

| Crate | Latest | Updated | Downloads | License | Notes |
|---|---|---|---|---|---|
| `iced` | 0.14.0 | 2025-12-07 | 2.85M | MIT | Elm-style, pure Rust, wgpu |
| `egui` | 0.36.2 | 2026-09-08 | 24.8M | MIT/Apache | immediate mode, fastest to ship |
| `slint` | 1.18.1 | 2026-09-21 | 1.79M | **GPL-3.0 OR royalty-free/commercial** | license friction with BSD-2 |
| `gtk4` | 0.11.5 | 2026-09-20 | 4.53M | MIT | needs C headers + `org.gnome.Platform` |
| `libadwaita` | 0.9.2 | 2026-07-07 | 2.37M | MIT | instantly native GNOME look |
| `tauri` | 3.0.0-alpha.3 | 2026-09-26 | 33.3M | MIT/Apache | WebKitGTK, heavy for this |
| `dioxus` | 0.8.0-alpha.1 | 2026-07-31 | 2.87M | MIT/Apache | alpha |
| `relm4` | 0.11.0 | 2026-04-08 | 1.13M | MIT/Apache | GTK4 + Elm |
| `floem` | 0.2.0 | **2024-11-14** | 43k | MIT | stale, skip |
| `gpui` | 0.2.2 | 2025-10-22 | 314k | Apache | Zed's; no tray, thin docs, churn |

Supporting crates:

| Need | Crate | Latest | License |
|---|---|---|---|
| Tray (StatusNotifierItem, pure Rust) | `ksni` | 0.3.6 | Unlicense |
| Tray (cross-platform, winit-oriented) | `tray-icon` | 0.26.0 | MIT/Apache |
| D-Bus | `zbus` | 5.19.0 | MIT |
| Secret Service / keyring | `secret-service` 5.2.0 / `oo7` 0.7.0-beta / `keyring` 4.2.0 | | MIT |
| SRP (official Proton, MIT) | `proton-srp` | 0.8.2 | **MIT** (ProtonMail/proton-crypto-rs) |
| Notifications | `notify-rust` | 4.18.1 | MIT/Apache |

### 3.2 Tray analysis (requirement #2)

- `tray-icon` pulls `ksni` as an optional Linux backend *or* `libappindicator` (C dep).
  Its non-optional deps include `muda` (menus) and it is designed around a winit event loop.
- `ksni` is **pure Rust over zbus** — zero C dependencies, works with *any* framework, or none.
  Just run it on its own thread.
- Host has `org.kde.StatusNotifierWatcher` and Ubuntu's appindicator extension → `ksni` works.
- **GNOME caveat:** GNOME has no built-in tray. Without the AppIndicator extension there is no
  tray at all. Ubuntu ships it (`ubuntu-appindicators@ubuntu.com`). On stock GNOME/Fedora the
  user must install it — must be documented, and the app must degrade gracefully
  (e.g. keep the window reachable, don't hide-to-nothing).

### 3.3 Flatpak analysis (deployment requirement)

> **Superseded:** Flatpak was subsequently dropped in favour of AppImage-only. The analysis is
> kept because it documents *why* — the sandbox fights both the host `protonvpn` CLI and
> tray-name ownership, and a wrapper gains nothing from it.

This is the part that constrains the design, and it is not obvious:

- Sandbox gives **no system D-Bus** by default → must add
  `--system-talk-name=org.freedesktop.NetworkManager` (and
  `--system-talk-name=me.proton.vpn.split_tunneling` to reuse split tunnelling).
- Tray needs to own a name outside our own namespace. The default session-bus policy only
  lets an app own `$FLATPAK_ID`. Existing Flathub apps (Signal, KeePassXC, Dropbox, Nextcloud,
  Zoom) work around this with `--own-name=org.kde.StatusNotifierItem-<pid>-<id>` or the broader
  `--own-name=org.kde.*`, plus `--talk-name=org.kde.StatusNotifierWatcher`.
  ([KDE bug 427625](https://bugs.kde.org/show_bug.cgi?id=427625))
- Keyring: `--talk-name=org.freedesktop.secrets`.
- GPU: `--device=dri`.
- Autostart: writing `~/.config/autostart/` from inside the sandbox is wrong; Flatpak has the
  **Background portal** (`org.freedesktop.portal.Background`, `RequestBackground`) for this.
- Flatpak 1.16.6 here → conditional permissions (`--socket-if=`, 1.17.0+) are unavailable.
- `org.gnome.Platform` is not installed → a GTK/libadwaita build means a bigger runtime pull
  than `org.freedesktop.Platform`.

AppImage is unconstrained (no sandbox) but must respect the oldest glibc it claims to support,
and bundles whatever C libs the GUI needs.

### 3.4 Recommendation

**Primary: `iced` 0.14 + `ksni` + `zbus`.** Pure Rust, MIT, no C toolchain deps, nothing to
bundle for AppImage beyond the binary, `org.freedesktop.Platform` for Flatpak, distinctive
custom theming possible (matches "красиво но минималистично"), and the tray is decoupled.

**Runner-up, if we prefer least-effort native looks: `gtk4` + `libadwaita`.** Instantly
"not ugly" on GNOME, tray via the already-installed `libayatana-appindicator3`, zbus integrates
with the GLib main loop. Costs: C dev headers, `org.gnome.Platform`, and the app will look like
every other GNOME app.

**Rejected:** `slint` (GPL-3.0/royalty-free licensing vs our BSD-2 repo — needs a lawyer, not
worth it), `floem` (stale), `gpui` (no tray, churn), `tauri`/`dioxus` (WebKitGTK weight is
absurd for a tray app), `egui` (fine, but its default look fights "неуёбищный").

**Caveat:** `iced` is pre-1.0 with breaking releases. The mitigation is architectural, not
framework choice — see `plan.md`: keep the engine in a UI-agnostic crate so the GUI is a thin,
replaceable layer. Since the UI is explicitly deferred ("обсудим потом"), pick the framework
at that point; nothing in the engine depends on it.

---

## 4. Licensing

Repo is **BSD-2-Clause**. Official Proton Linux stack is **GPL-3.0**.

- Calling `protonvpn` as a separate process is fine, but it is not a design we want anyway.
- Copying/porting `proton-vpn-api-core` or `local-agent-rs` code into this repo is **not**
  acceptable → any reimplementation must be clean-room, from observed protocol/API behaviour.
- Good news: Proton's **Rust** crates (`proton-crypto-rs`: `proton-srp`, `proton-crypto`,
  `proton-crypto-account`) are **MIT** and actively maintained — safe to depend on for auth.
- Reading the user's own keyring session is a data-access question, not a copyright one.
