# The application's own name. A brand: it is the same in every language.
chrome-app-name = Proton VPN

# A language's name for itself, shown in the language picker in Settings → Application. It is
# always written in the language it names, never translated into the one currently on screen —
# that is what makes the picker usable to somebody stuck in a language they cannot read.
chrome-locale-name = English

# Shown on Settings where the version number normally goes, when this build was not made as a
# release. A version number is data and is never translated; this sentence is ours.
chrome-version-unversioned = a build that is not from a release

# The line under the application's name in the sidebar's brand block. It is the window's only
# explanation of what this program is. Small and faint (11 px) in a 238 px rail.
chrome-brand-subtitle = a wrapper around the CLI

# The sidebar's account block when `protonvpn info` has not named an account yet — which is not the
# same as "signed out", so the wording must not claim it is. One faint line in the 238 px rail,
# above the Sign in button.
chrome-account-unnamed = No account yet

# The button in the sidebar's account block that opens the sign-in form. Small: 12 px in a compact
# button, directly under the faint "No account yet" line.
chrome-sign-in = Sign in

# The button beside the account name in the sidebar that signs the account out. Small: 12 px in a
# compact button, at the right end of the row that holds the avatar and the name.
chrome-sign-out = Sign out

# The button that dismisses the notice bar — the strip across the top of the window that carries our
# own remarks, never the CLI's output. Very small: 12 px, at the right end of the bar.
chrome-notice-dismiss = ok

# The Overview page's name in the sidebar's navigation. One word, in the 238 px rail, with a small
# accent rule drawn to its left.
chrome-page-overview = Overview

# The Settings page's name in the sidebar's navigation. One word, same rail.
chrome-page-settings = Settings

# A tab above the Settings page's cards. Five tabs share one row, so each name is one short word.
chrome-tab-general = General

# A tab above the Settings page's cards, next to General. Holds our own settings and the CLI's.
chrome-tab-connection = Connection

# A tab above the Settings page's cards. The local SOCKS5 proxy.
chrome-tab-proxy = Proxy

# A tab above the Settings page's cards. How often the CLI is asked for `status`.
chrome-tab-polling = Polling

# A tab above the Settings page's cards. The account, and the update.
chrome-tab-account = Account

# The name of the first system preset in the Overview page's connection list. A system preset is one
# of the CLI's own shortcuts, not a saved profile: it has no country and cannot be edited.
chrome-preset-fastest = Fastest

# The one line under the Fastest preset's name. It must not promise speed: the CLI picks the least
# loaded server, which is what this says. Short — one line in the connection list.
chrome-preset-fastest-summary = Least loaded server

# The name of the Secure Core system preset. Proton's own product name — leave it as it is.
chrome-preset-secure-core = Secure Core

# The one line under the Secure Core preset's name: traffic enters through a hardened country first.
chrome-preset-secure-core-summary = Double hop

# The name of the P2P system preset. The CLI's own flag name — leave it as it is.
chrome-preset-p2p = P2P

# The one line under the P2P preset's name.
chrome-preset-p2p-summary = P2P servers

# A pill on a connection row: this route goes through Secure Core. Drawn small and uppercase
# (10 px), so keep it short. Proton's own product name — leave it as it is.
chrome-badge-secure-core = SECURE CORE

# A pill on a connection row: the CLI's `--p2p` flag is set. The CLI's own flag name — leave it.
chrome-badge-p2p = P2P

# A pill on a connection row: the CLI's port-forwarding flag is set. One short word, uppercase at
# 10 px. This is our word, not a flag name, so translate it.
chrome-badge-port = PORT

# A checkbox in the connection editor. The name is the CLI's own flag name — leave it as it is.
chrome-editor-flag-p2p = P2P

# A checkbox in the connection editor. Proton's own product name — leave it as it is.
chrome-editor-flag-secure-core = Secure Core

# A checkbox in the connection editor. The name of the Tor network — leave it as it is.
chrome-editor-flag-tor = TOR

# A checkbox in the connection editor: ask the CLI for a forwarded port as well. The CLI's own name
# for the setting. The longest of the four, and it shares their row in the editor.
chrome-editor-flag-port-forwarding = Port forwarding

# A notice in the bar across the top of the window. It appears when the window's close button was
# pressed and there is no StatusNotifierItem host (docs/architecture.md §9): the window cannot be
# hidden into nothing, so it stays and says so. "Quit" names the window's own button (tray-quit),
# capitalized because it names that button.
chrome-notice-no-tray-close = There is no tray to hide the window in. To close the application, use Quit.

# The same situation reached from the other end: hiding was asked for from the tray's own menu, or a
# run that asked to start in the tray waited for a panel that never appeared. One short line in the
# notice bar.
chrome-notice-no-tray-hide = There is no tray — the window stays open.

# A notice after Save was pressed on a connection whose name field was empty. A connection cannot do
# without a name; nothing is written.
chrome-notice-connection-needs-name = A connection needs a name.

# A notice after Apply was pressed on a CLI setting whose text field was empty. The empty value is
# refused rather than sent, because the CLI would read it as "unset".
chrome-notice-empty-value = A value cannot be empty.

# A notice after Sign in was pressed with the username or the password missing. The password itself
# never leaves the form.
chrome-notice-login-required = A username and a password are required.

# A notice after the two-factor form was submitted empty. The form waits for the CLI's own prompt;
# this is only the refusal to send nothing.
chrome-notice-two-factor-required = Enter the code from your authenticator app.

# A notice on Settings → Proxy when the port field is not a number. The field keeps what was typed.
# 0 and 65535 are the ends of the range a TCP port is written in, and are data.
chrome-notice-proxy-port = Proxy port: a number from 0 to 65535 is required.

# A notice on Settings → Proxy when the verification interval is not a number of seconds. 0 turns
# the check off, which is why it is spelled out.
chrome-notice-proxy-verify = Tunnel check: a number of seconds is required (0 turns it off).

# A notice on Settings → General when `custom-dns` was switched on with the server list still empty.
# `custom-dns` and `on` are the CLI's own key and value — leave them exactly as they are.
chrome-notice-custom-dns = custom-dns on needs servers: without them the CLI rejects the command.

# A notice in the bar when the autostart entry could not be written. $reason is the operating
# system's own error text — data, shown verbatim and never translated.
chrome-notice-autostart-failed = autostart: { $reason }

# A notice in the bar when the application-menu entry or its icon could not be installed. $reason is
# the operating system's own error text — data, shown verbatim and never translated.
chrome-notice-entry-failed = menu entry: { $reason }

# The second half of a saved connection's one-line summary when it names a country but no city:
# `Switzerland · any city`. One short line under the connection's name.
chrome-summary-any-city = any city

# The whole summary of a saved connection that names no country at all: the CLI will decide where to
# connect. Drawn in the same one-line slot as a country name.
chrome-summary-any-country = Any country
