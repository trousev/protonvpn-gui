# The heading of the language block, first on Settings → General. A user who has landed in a
# language they cannot read has to be able to find their way out without reading anything.
settings-language-title = Language

# The line under the language heading. It explains what leaving the picker alone does, because
# "System" is not self-explanatory and the variables are the honest answer.
settings-language-hint = Follows the desktop unless you choose here: the LANGUAGE, LC_ALL, LC_MESSAGES and LANG variables, in that order, and English when none of them names a language this build carries.

# The first entry of the language picker: use whatever the desktop asks for.
settings-language-system = System

## The page itself.

# The small line above the page title, at the top of Settings. It is the page's own name, drawn in
# capitals by the widget rather than by the string — write the word a heading would use.
settings-eyebrow = Settings

# The page title, 22px, in the header row beside the refresh button. Every card below holds keys of
# `protonvpn config list`; the title says whose settings they are. `CLI` is that program and stays
# as it is.
settings-heading = CLI settings

# The button that runs `protonvpn config list` again, in the page header and again in the empty
# state of the CLI card. `config list` is the CLI's own subcommand — a command line, never
# translated.
settings-refresh = Read config list

# The empty state of the CLI card: nothing has been read yet. Two short sentences, calm rather than
# alarming — this is the first thing on the page and it is not a failure.
settings-unread = The CLI settings have not been read yet. One command, about a second, and what the CLI answered appears here.

# The description of a key `protonvpn config list` printed and this application does not describe.
# The key itself is shown beside this line, verbatim — it is data.
settings-unknown-key = A key this application does not describe: shown as it is.

# The grey placeholder inside the free-form value field. It appears twice: on the CLI card for a key
# with no list of values, and on the Account tab where an unrecognised prompt is answered. One word.
settings-value-placeholder = value

# The button that writes one value — `protonvpn config set` for a setting, and the same word for
# saving the proxy's address, port and check interval. One word in a narrow button.
settings-apply = Apply

# The label of the custom DNS server list, drawn beside the field only while `custom-dns` is on.
# The servers themselves are addresses and are never translated.
settings-dns-servers = Servers

## The words for the values `protonvpn config list` prints. The value is data — `off`,
## `malware-only`, `standard` — and is never translated; these are our words for it, so that nobody
## has to read `malware-ads-trackers` in a dropdown.

# The `off` value of NetShield, in the Connection tab's dropdown. NetShield is Proton's own product
# name — leave it exactly as it is. This is deliberately not the same message as the other `off`s:
# in a language with gender it agrees with NetShield, not with a generic setting.
settings-value-netshield-off = Off

# NetShield's middle value: malicious domains only.
settings-value-netshield-malware-only = Malware only

# NetShield's strongest value: malicious domains, ads and trackers together. The longest option in
# the dropdown, which is 240px wide.
settings-value-netshield-malware-ads-trackers = Malware, ads and trackers

# The `off` value of the kill switch. A separate message for the same reason as NetShield's: it
# agrees with "kill switch". `Kill switch` is Proton's own product name — leave it.
settings-value-kill-switch-off = Off

# The kill switch's on value, which the CLI spells `standard`. A word, not the CLI's value.
settings-value-kill-switch-standard = Standard

# The generic `on` in a dropdown, for a setting this page names no more precisely. Neuter where the
# language has gender, unlike the NetShield and kill-switch words above.
settings-value-on = On

# The generic `off` in a dropdown, for a setting this page names no more precisely.
settings-value-off = Off

## The application's own switches, on the General and Connection tabs.

# The heading of the block holding our own switches. It heads the second half of the General tab,
# and it heads the whole Connection tab — one word, an eyebrow.
settings-app-section = Application

# Autostart. The label is one short line, the description sits under it in muted text.
settings-autostart-label = Start at login

# What the autostart switch actually does: it keeps one file in step with itself. The path is data
# and is never translated.
settings-autostart-hint = Keep ~/.config/autostart/protonvpn-gui.desktop in step with this setting.

# The .desktop entry switch: its label, one short line. The `.desktop` file it installs is a file
# format and stays as it is.
settings-desktop-entry-label = Entry in the application menu

# The description of the .desktop entry switch — the longest on this page. It explains a design
# decision rather than a preference: on Wayland a window has no icon of its own. Do not soften it
# into "adds a menu shortcut" — that is exactly the guess it exists to prevent.
settings-desktop-entry-hint = Install the .desktop file and the icon into ~/.local/share. This is not decoration: on Wayland a window has no icon, and the desktop learns the window's name and icon only from the .desktop file — without it GNOME shows the window as "Unknown application".

# Start hidden in the tray: the label, one short line.
settings-start-minimized-label = Start minimised in the tray

# What starting minimised means: no window at all until the tray is clicked.
settings-start-minimized-hint = Start with no window: the application lives in the tray from the first moment.

# The line under the switches saying what is on disk right now, and where our own settings live.
# $autostart and $entry are the words from settings-yes and settings-no. The two paths are data and
# are never translated.
settings-files-line = Autostart: { $autostart } · menu entry: { $entry } · the application's own settings live in ~/.config/protonvpn-gui/config.json. We neither read nor write the official application's files.

# The word for "present on disk", used only inside settings-files-line. One word.
settings-yes = yes

# The word for "not present on disk", used only inside settings-files-line.
settings-no = no

# Connect at startup, on the Connection tab: the label of the switch, one short line.
settings-connect-at-startup-label = Connect at startup

# The description of that switch. It carries a design decision: a live tunnel is not rebuilt, because
# `connect` against a live connection silently switches servers. Keep that sentence — it is why the
# switch behaves the way it does.
settings-connect-at-startup-hint = Brings the selected connection up right after the start, including when there is no window and the application lives in the tray. If the CLI already reports a connection the tunnel is left alone: `connect` against a live connection silently switches servers.

# Under that switch: which connection a startup connect would use, and the command it would run.
# $name is a saved connection's name as the user typed it, or a preset's name — data, never
# translated. $argv is the command line, verbatim, and is data too.
settings-selected-connection = Selected now: { $name } · $ { $argv }

## The polling card, on the Polling tab.

# The heading of the card: how the application learns the state at all.
settings-polling-eyebrow = How we learn the state

# The first line of the polling card. `protonvpn status` is a command line and is never translated;
# the sentence is the honest reason the polling looks the way it does.
settings-polling-intro = protonvpn status costs about a second: every call starts a Python interpreter. So the polling is built like this:

# The first of three bullets: the idle cadence. A fragment ending in a semicolon — keep the list
# parallel.
settings-polling-idle = the calm cadence — no more often than once every 5 minutes;

# The second bullet: polling right after a command that could have changed the tunnel.
settings-polling-after-command = right after a command that could have changed the tunnel;

# The third bullet: polling when the user is looking.
settings-polling-when-you-look = when you are looking — when the window opens or the tray is clicked.

# The note at the end of the card. $age is already a finished fragment in the current language
# ("updated 3 mins ago") and is not translated again.
settings-polling-consequence = The consequence, which we do not hide: the state can be old. The age always stands beside the status: { $age }.

# The curl probe switch: its label, one short line. `curl` is a program name and stays as it is.
settings-probe-label = Check the egress address with curl

# The description of that switch. It names the probe as sanctioned exception #1: the only external
# call besides protonvpn. `protonvpn` is a program name and stays as it is.
settings-probe-hint = Exception #1: the only external call besides protonvpn. It answers a question the CLI cannot — whether traffic is going through the tunnel.

## The account card, on the Account tab.

# The heading of the account card.
settings-account-eyebrow = Account

# Shown when `protonvpn info` named an account. $name is the account name the CLI printed — data,
# never translated. The sign-out button sits at the other end of the row.
settings-account-signed-in = Signed in as { $name }

# The sign-out button, beside the account name. One word in a narrow outlined button.
settings-account-logout = Sign out

# Shown when `protonvpn info` did not name an account. `protonvpn info` is a command line and stays
# as it is; the backticks are literal.
settings-account-unconfirmed = The account is not confirmed: `protonvpn info` did not name one.

# The sign-in button beside that sentence. One word.
settings-account-signin = Sign in

# The explanation under the account row, shown whether or not anyone is signed in. It is a promise
# about the password and the 2FA code, so keep it exact.
settings-account-secret-hint = The password and the 2FA code go straight into the CLI process's PTY terminal and never reach the console: the console shows only what the CLI printed.

# Shown while the CLI waits for an answer we recognised. $prompt is the word from one of the three
# messages below.
settings-prompt-waiting = The CLI is waiting for input: { $prompt }

# What the CLI is waiting for, after the colon above. One word.
settings-prompt-password = password

# The one-time code from an authenticator application.
settings-prompt-two-factor = 2FA code

# A question we could not classify. It is not an error: the question itself is readable in the
# console, and this says so.
settings-prompt-unrecognised = an unrecognised request — see the console

# The grey line where the manual prompt would appear, when there is none. It says when the field
# shows up and where the question itself can be read.
settings-account-manual-hint = If the CLI asks a question we could not recognise, the field appears here and the question itself is visible in the console.

# The button that sends the typed answer into the PTY. One word in a narrow button.
settings-send = Send

## The local SOCKS5 proxy — exception #3 (docs/architecture.md §13).

# The heading of the proxy card. `SOCKS5` is a protocol name and is never translated.
settings-socks5-eyebrow = Application · SOCKS5

# The card's own explanation. It is a design decision and not a sales line: the proxy refuses
# everything until the tunnel is proven, and it is off by default. Keep both facts.
settings-socks5-intro = A local SOCKS5 proxy for programs that must reach the network only through the VPN: point the application at this address, and the proxy refuses everything until it is proven that traffic goes through the tunnel. Off by default.

# The checkbox that turns the proxy on.
settings-socks5-enable = Enable the local SOCKS5 proxy

# The label of the address field, drawn beside it. One word. The address in the field is a loopback
# address and is data.
settings-socks5-address-label = Address

# The label of the port field. One word.
settings-socks5-port-label = Port

# The label of the field holding how often the tunnel is verified, in seconds. Narrow: it sits
# between the port field and the Apply button, so keep it short. A `0` in the field turns the
# external check off.
settings-socks5-verify-label = Tunnel check, s

# The state line when the gate is open. $source is the local IPv4 address the kernel answers the
# route question with — data, never translated.
settings-socks5-open = open · route confirmed: { $source }

# The state line when the gate is closed. $reason is a finished sentence from the catalogue
# (`proxy-gate-*`: "VPN is not connected", "route 10.0.0.2 is unproven…") and is already in the
# reader's language — do not translate it again, and keep the separator.
settings-socks5-closed = closed · { $reason }

# Advice under the state line, when the proxy is on but the CLI reports no connection: nothing is
# wrong and the gate opens by itself. One sentence.
settings-socks5-advice-not-connected = The proxy will open on its own as soon as the CLI reports a connection.

# Advice when the kernel's route was never observed before connecting, so there is nothing to
# compare it against. The two button names in it — Disconnect, then Connect — are the words of the
# Overview page; translate them the same way here.
settings-socks5-advice-unverified = The application never saw what the route was before connecting, and cannot claim the current one is the tunnel. Reconnect (Disconnect, then Connect): then the route will be confirmed.

# Advice when the proven route changed or disappeared. Direct, not alarmist: this is the proxy doing
# its job, and it will not relay until the tunnel is proven again.
settings-socks5-advice-route = Reconnect to confirm the tunnel again: until that is done the proxy will not let a single byte out.

# Advice when the egress check reports the pre-connection address again: the tunnel is not carrying
# traffic, whatever the CLI says.
settings-socks5-advice-egress = The egress check saw the same address as before connecting. Reconnect.

# Advice when the background tunnel check stopped answering. It says exactly what is left — the
# local route check — and what that check cannot see.
settings-socks5-advice-probe-silent = The external tunnel check is not answering: without it only the local route check is left, and it does not see everything. Check the connection and reconnect, so that the tunnel is confirmed again.

# Advice when there is no listener: the address is not loopback, or the port is taken. `localhost`
# is a host name and stays as it is.
settings-socks5-advice-not-listening = Check the address and the port: only localhost can be listened on.

# The button that copies the proxy address to the clipboard. Narrow and outlined.
settings-socks5-copy = Copy address

# The confirmation shown for two seconds after the address was copied. $address is the address that
# was copied — data, never translated.
settings-socks5-copied = { $address } copied

# The grey line telling the reader what to point their program at. $address is the listening address
# and port — data. `SOCKS5` is the protocol name.
settings-socks5-point-at = Point the application at { $address } (SOCKS5, no authentication).

# The counters line under the copy button. Every number is a counter from the proxy, and the two
# byte counts arrive already written in this language by human_bytes ("4.9 MiB"). The numbers, the
# units and the slash are data; the five words around them are ours.
settings-socks5-counters = accepted { $accepted } · refused { $refused } · active { $active } · transferred { $up } / { $down }

# Appended to the counters line when sessions were turned away because the session limit was
# reached. $overloaded is that count — a number, so that a language with three plural forms can
# decline the word after it. It starts with the separator that joins it to the line above.
settings-socks5-overloaded = { $overloaded ->
        [one] · no room left for { $overloaded } more session
       *[other] · no room left for { $overloaded } more sessions
    }

# The grey line shown when the curl probe is off: it names exactly what is lost and what remains.
# `curl` is a program name; "the Polling tab" is this page's own tab, the one called Polling.
settings-socks5-probe-off = The external tunnel check is off, along with the egress check on the Polling tab: only the local route check remains, every 200 ms.

# The grey line about what the proxy refuses. `localhost`, `IPv4`, `CONNECT` and `ssh -D` are facts
# about the protocol and the comparison, and are never translated.
settings-socks5-loopback = localhost only and IPv4 only; of the SOCKS5 commands, only CONNECT. There is no authentication: the port listens on the loopback interface, exactly as `ssh -D` does.

# The last grey line: how the route is watched, and what the seconds field does. `curl` is a program
# name, `200 ms` is a measurement, and `0` is the value the field takes.
settings-socks5-watchdog = The kernel's route is re-read every 200 ms — no packets and no third parties. Once per the given number of seconds the tunnel is confirmed by its egress address, with the same curl check as on the Overview page; 0 turns that off and the local check remains.

## The AppImage updater — exception #4 (docs/architecture.md §14).

# The four update policies, drawn as a row of radios in this order: from "leave me alone" to "do
# everything". Four in one row, so keep them short.
settings-update-policy-off = do not check

# Report a new release and do nothing else.
settings-update-policy-notify = notify only

# Download and verify it, but do not put it in place.
settings-update-policy-download = download

# Download it and put it in place; it takes effect at the next start.
settings-update-policy-install = download and install

# The heading of the updater card, first on the General tab. `AppImage` is a package format and
# stays as it is.
settings-updates-eyebrow = Application · Updates

# The card's explanation. It carries the design decision that nothing downloaded is ever executed,
# and that the previous image is kept as <name>.old for one run. Keep both facts; do not soften them
# into "keeps itself up to date".
settings-updates-intro = Nothing updates an AppImage but itself: the application asks its own release page, checks what it downloaded against the SHA256SUMS of the same release, and swaps the file in place. What it downloaded is never executed by itself — the new version takes effect at the next start, and the previous one stays beside it as <name>.old for one run.

# The line saying which version this build is. $version is a version number, or the sentence from
# chrome-version-unversioned — data either way, never translated. One of the two messages below is
# appended to it, after a " · " separator.
settings-updates-version = Version: { $version }

# Appended to the version line: when the release page was last asked. $age is a finished fragment
# from the age table ("updated 3 mins ago") and is not translated again.
settings-updates-page-age = release page: { $age }

# Appended to the version line when the release page has never been asked.
settings-updates-page-never = the release page has not been asked yet

# The button that asks the release page right now. It is disabled while a check or a download runs.
settings-updates-check-now = Check now

# The button that stops a download in progress. One word.
settings-updates-cancel = Cancel

# The button that puts an already downloaded, verified image in place.
settings-updates-install = Install

# The button that downloads the newer release and puts it in place. Longer than the one above; the
# two never appear together.
settings-updates-download-install = Download and install

# The button that stops the update being mentioned for this version. Small and quiet, on the same
# row as the others.
settings-updates-dismiss = Do not remind me

# The grey line under the buttons: when the automatic check runs, and what the policy does or does
# not do. The claim that a running process is untouched is the honest part — keep it.
settings-updates-footnote = The check runs once a day and ten seconds after the start; the policy decides what the application does by itself, and the buttons always work. A new version takes effect after a restart: the swap does not touch a process that is already running.

# The note shown when the last check or download failed. $error is the updater's own message — data,
# never translated.
settings-updates-error = The last attempt failed: { $error }

# The note shown when this build cannot replace itself: a tarball install, or `cargo run`. It says
# what to do instead.
settings-updates-not-replaceable = This build is not an AppImage (or is not running from the image): it cannot replace itself. The update has to be downloaded from the release page by hand.

# The last grey line: what the checksum does and does not prove. The honesty is deliberate — a
# checksum does not prove authorship. `SHA256SUMS`, `GitHub`, `gh attestation verify` and
# `SECURITY.md` are names and are never translated.
settings-updates-sums = SHA256SUMS covers a broken transfer, corruption and a mirror serving yesterday's image, but not a substitution on GitHub's side: authorship is verified separately, `gh attestation verify` — SECURITY.md.

# The updater's state line: one message per state. The two a status line usually gets wrong are
# spelled out — "downloaded" is not "installed", and "installed" is not "running".

# The release page is being asked right now. Short, with a trailing ellipsis.
settings-update-checking = asking the release page…

# A download in progress, with the size known. $received and $total arrive already written in this
# language by human_bytes ("4.9 MiB") and are data.
settings-update-downloading = downloading { $received } of { $total }

# The same, while the release did not say how large the asset is.
settings-update-downloading-unknown = downloading { $received }

# The image is downloaded, checked against SHA256SUMS, and waiting for a restart. $version is a
# version number — data, never translated.
settings-update-staged = { $version } downloaded and verified — waiting for a restart

# The image is already in place and takes effect at the next start. $version is a version number.
settings-update-installed = { $version } is in place — it will take effect after a restart

# The release page has never been asked.
settings-update-never-checked = not checked yet

# The release page answered, and the latest release has no image we could install. Deliberately not
# the same answer as "not checked yet".
settings-update-no-image = the latest release has no image I could install

# A newer release exists. $latest is a version number — data, never translated.
settings-update-available = { $latest } is available

# This build is newer than the latest release, which is what a development build sees.
settings-update-ahead = { $current } is newer than the latest release ({ $latest })

# This build is the latest release it knows about. $current is a version number.
settings-update-current = { $current } is the latest version

# A build with no version of its own, compared against the latest release. $latest is a version
# number.
settings-update-versionless = latest release: { $latest }; this build has no version

## What `protonvpn config list` calls its keys, in words a person reads: one name and one
## description per key. The key itself — `netshield`, `kill-switch` — is data and stays verbatim
## beside the name. Proton's own product names (NetShield, Kill switch, VPN Accelerator,
## Moderate NAT, IPv6) are the same in every language and are marked where they occur.

# NetShield. Proton's own product name — leave it exactly as it is in every language.
settings-name-netshield = NetShield

# What NetShield does, under its name on the Connection tab.
settings-hint-netshield = Block malicious domains at the gateway's DNS level.

# The kill switch. Proton's own product name — leave it as it is.
settings-name-kill-switch = Kill switch

# What the kill switch does. Short: one line under the name.
settings-hint-kill-switch = Block traffic if the tunnel drops.

# Port forwarding. The description says who holds the lease, because that is our own behaviour —
# the CLI hands out the port and we renew the mapping. `NAT-PMP` is a protocol name and stays.
settings-name-port-forwarding = Port forwarding

# What port forwarding does, and who keeps the lease alive.
settings-hint-port-forwarding = Let the server hand out a forwarded port. We hold the lease ourselves, over NAT-PMP.

# Custom DNS servers.
settings-name-custom-dns = Custom DNS

# What custom DNS does. The servers themselves are addresses and are never translated.
settings-hint-custom-dns = Use the given DNS servers inside the tunnel.

# VPN Accelerator. Proton's own product name — leave it as it is.
settings-name-vpn-accelerator = VPN Accelerator

# What VPN Accelerator does. One short line.
settings-hint-vpn-accelerator = A speed-up on distant servers.

# Moderate NAT. Proton's own product name — leave it as it is.
settings-name-moderate-nat = Moderate NAT

# What Moderate NAT does. `P2P` is Proton's own name for that kind of server and stays.
settings-hint-moderate-nat = Softer NAT for games and P2P.

# IPv6. A protocol name, the same in every language. The setting allows IPv6 inside the tunnel.
settings-name-ipv6 = IPv6

# What the IPv6 setting does.
settings-hint-ipv6 = Carry IPv6 inside the tunnel.

# Anonymous crash reports.
settings-name-anonymous-crash-reports = Anonymous crash reports

# What that setting does: crash logs are sent, but not linked to the account.
settings-hint-anonymous-crash-reports = Send crash logs without linking them to the account.
