# The eyebrow above the Overview page's title: the small caption that names the page. It is drawn
# in capitals by the widget, so write it the way it should read and do not capitalise it here.
overview-page-eyebrow = Overview

# The Overview page's title, right under that eyebrow. It is the largest text on the page and the
# page's own subject, not the tunnel's state: the state is the card below it.
overview-page-title = Connection

# The button at the top right of the Overview page. It asks the CLI for a fresh `status`; it does
# not connect anything. Short: it shares its row with the page title.
overview-refresh-status = Refresh status

# The big line in the status card when the CLI reports a live tunnel. It is the server and where it
# is, as the CLI printed them — `NL#818 in Amsterdam, Netherlands`.
# $server is a server name and $location is the CLI's own location string; both are data and
# neither is translated.
overview-status-connected = { $server } in { $location }

# The big line in the status card while a connect is in flight. It claims the intent only: the CLI
# has not answered yet, so nothing about the tunnel is known (docs/architecture.md §5).
overview-status-connecting = Connecting…

# The big line in the status card when the CLI says there is no tunnel.
overview-status-disconnected = No active tunnel

# The big line in the status card after the CLI refused or failed. The CLI's own message is shown
# verbatim in the note underneath, so this line says only whose answer it was.
overview-status-error = The CLI refused

# The big line in the status card before the CLI has answered anything at all. This is not an error
# and it is not "disconnected": nothing is known yet.
overview-status-unknown = The state is unknown

# The note under that line, and the reason it exists: an unknown state is shown as unknown instead
# of being guessed at (docs/architecture.md §5).
overview-status-unknown-note = The CLI has not answered yet — the state is not invented.

# The status card's main button while a tunnel is up. It tears the tunnel down.
overview-disconnect = Disconnect

# The status card's main button when there is no tunnel. It connects the selected connection: a
# request to the CLI, not a promise about the outcome.
overview-connect = Connect

# One of the four small tiles under the status line. The widget draws the label in capitals and
# puts a value beside it; the value is data (a server name, a city, a percentage, a protocol) and is
# never translated. "Server" means the CLI's own server identity, not this application.
overview-tile-server = Server

# The second tile: the city half of the CLI's location string. Data beside it.
overview-tile-city = City

# The third tile: the server's load in percent, or an em dash when the CLI did not report one.
overview-tile-load = Load

# The fourth tile: the protocol the CLI reported, spelled the CLI's way (data).
overview-tile-protocol = Protocol

# The line under the status title: how old this state is, and which connection the Connect button
# is aimed at. $age is already a translated phrase ("updated 3 mins ago"); $target is the selected
# connection's name — one of the CLI's presets or a profile the user named — and is data.
overview-age-and-target = { $age } · connection: { $target }

# The verdict beside the egress card's eyebrow when the measured address differs from the one
# measured before connecting. Short and lowercase: it shares its row with the eyebrow and a button.
overview-egress-changed = address changed

# The verdict beside that eyebrow when the address did not change. It is not a success — the note
# below it explains what it means.
overview-egress-unchanged = address did not change

# The eyebrow of the egress probe card. An eyebrow: drawn in capitals by the widget.
# "egress" is the address traffic leaves by; "ground truth" is deliberate — a measurement is the
# evidence, and the CLI's own report is not (docs/cli-surface.md §4.9).
overview-egress-eyebrow = Egress probe · ground truth

# The egress card's button: run the probe now.
overview-egress-measure = Measure

# A fact row in the egress card: the address the probe measured. The value beside the label is an
# IP address — data, never translated. "probe" distinguishes it from any address the CLI reports.
overview-egress-ipv4 = IPv4 (probe)

# A fact row in the egress card: the address measured before connecting, which the probe is
# compared against.
overview-egress-baseline = Before connecting

# A fact row in the egress card: the country the GeoIP service reported. "(advisory)" is the
# honesty — the geo databases disagree with each other, so this is not evidence of anything.
overview-egress-country = Country (advisory)

# A fact row in the egress card: the autonomous system and organisation the service reported. The
# value is data; "ASN" is the standard abbreviation and stays as it is.
overview-egress-asn = ASN / organization

# A fact row in the egress card: which service answered the probe. The value is data (a host name).
overview-egress-source = Source

# The note under the egress card's rows when the address changed: the tunnel is carrying traffic.
overview-egress-changed-note = The address changed — traffic is going through the tunnel.

# The note under those rows when the address did not change. This is the honest reading of a live
# tunnel that carries nothing, and it says so without softening
# (docs/architecture.md §13).
overview-egress-unchanged-note = The address did not change. If the CLI says "connected", the tunnel is not carrying traffic.

# The note under those rows when there is a current reading but no baseline to compare it against —
# the address was read before the application started measuring, so it proves nothing.
overview-egress-no-baseline = There is nothing to compare against: this address was read before we started measuring.

# The note under those rows before anything has been measured: the probe runs after connecting, so
# there is no tunnel to measure through yet.
overview-egress-no-tunnel = The probe runs after connecting — the tunnel is not active.

# The faint paragraph at the bottom of the egress card: why this card may look redundant beside the
# status. It is a design decision written out, and it stays blunt — the CLI can be wrong about the
# egress address, so the measurement outranks it.
# "ifconfig.co/json" is a URL and "protonvpn" is the program we run: both are data, not words.
overview-egress-provenance = The probe to ifconfig.co/json is the only external call besides protonvpn. The CLI can be wrong about the egress address, so the ground truth is a measurement, not a self-report. Country and ASN are advisory only: GeoIP databases disagree with each other.

# The eyebrow of the connections card, which lists the CLI's presets and the user's profiles. An
# eyebrow: drawn in capitals.
overview-connections-eyebrow = Connections

# The button in that card's header, at the top right: it opens the profile editor on a new profile.
overview-connections-add = Add connection

# The faint caption above the three presets. They are the CLI's own shortcuts and cannot be
# renamed, edited or deleted — this line is why they have no Edit and Delete buttons.
overview-connections-system = System · not editable

# The faint caption above the user's own profiles, below the presets.
overview-connections-mine = My connections

# Shown in place of the profile list when the user has none. "Add connection" is the name of the
# button above it, written out because a message may not refer to another message.
overview-connections-empty = No connections of your own yet. "Add connection" builds a profile: country, city, P2P, Secure Core, Tor and port forwarding.

# The faint line at the bottom of the connections card: whatever is selected, the command is always
# a connect for it.
# "protonvpn connect" is the argv of the command we will run — data, never translated.
overview-connections-footer = Connecting always runs for the selected connection: protonvpn connect.

# The rename-and-edit button on one of the user's own profile rows. Not a preset row: those have no
# such button. Short, and one of a pair with Delete beside it.
overview-connection-edit = Edit

# The delete button on one of the user's own profile rows, beside Edit. Destructive and short; the
# row it belongs to is already on screen, so the label does not repeat the profile's name.
overview-connection-delete = Delete

# The port-forwarding card's header label, at the right end of the row that carries the eyebrow:
# port forwarding is off for the selected connection, so there is nothing to show. Lowercase, one
# line, narrow — the eyebrow on its left is long.
overview-port-off = off for this connection

# The same header label while a NAT-PMP lease is held.
overview-port-active = lease active

# The same header label while a lease has been asked for and the gateway has not answered yet.
overview-port-pending = asking for a lease

# The same header label when the connected server cannot forward a port at all. "it" is the server
# feature the card is about; the sentence is a label, not an explanation — that is below.
overview-port-unsupported = the server does not support it

# The same header label when the lease failed or cannot be held.
overview-port-unavailable = unavailable

# The same header label when the card is on but no lease has been asked for yet.
overview-port-idle = no lease

# The eyebrow of the port-forwarding card. An eyebrow: drawn in capitals. "Port forwarding" is the
# feature; the card is about the NAT-PMP lease that provides it.
overview-port-eyebrow = Port forwarding

# The button beside the port number that copies it to the clipboard. Short: the number itself is
# drawn large next to it.
overview-port-copy = Copy

# The line under the port number: how long the current lease lasts and that it is renewed without
# being asked. $seconds is the lease lifetime in whole seconds — a number, not data to translate.
overview-port-lease = lease { NUMBER($seconds) } s, renewed automatically

# Shown for a couple of seconds after the copy button is pressed, in place of the line below.
overview-port-copied = Port copied to the clipboard

# The line under the port number while a lease is held, after the copy confirmation has faded: the
# port is not stable, and the user is about to paste it somewhere.
overview-port-volatile = The gateway hands out the port, and it changes after a reconnect.

# The note beside the em dash when a lease could not be obtained. It is a design decision and it
# stays this blunt: a number nobody renews would be worse than no number.
overview-port-hidden = The port is deliberately not shown: showing a number nobody is renewing would be misleading.

# The line under the em dash while the gateway is being asked, after the request was sent.
# "NAT-PMP" is a protocol name and stays as it is.
overview-port-requesting = Asking the gateway for a lease over NAT-PMP…

# The line under the em dash when the connected server does not support port forwarding. "P2P" is
# the CLI's own term for the servers that do.
overview-port-unsupported-note = This server does not support port forwarding. Connect to a P2P server.

# The line under the em dash when port forwarding is on for the profile but there is no lease at
# all — nothing has been asked for yet.
overview-port-none = There is no lease.

# The label of the toggler that decides whether the selected profile asks for a lease. It appears
# only for a profile the user owns; the presets have no such switch.
overview-port-keep = Keep a lease for this profile

# Shown instead of that toggler when a system preset is selected: presets are fixed, and a profile
# is how a user gets a setting of their own. "Add connection" is the button's label in the
# connections card, written out because one message may not refer to another.
overview-port-system-preset = System presets are not editable: a profile is created with the "Add connection" button.

# The button that asks the gateway for a lease again, used when the automatic request failed.
overview-port-refresh = Request again

# The button that gives the current lease up. It sits beside Request again, in the muted style
# because it is not an emergency.
overview-port-release = Release

# The faint paragraph at the bottom of the port-forwarding card: where the number comes from, and
# why a port is sometimes withheld. It is a design decision written out and stays exact.
# "10.2.0.1:5351" is an address, "RFC 6886" is a document number and "opcode 0" is a field value —
# all data, all shown as they are.
overview-port-provenance = The port comes from a NAT-PMP lease at 10.2.0.1:5351 (RFC 6886), and only when the selected connection asks for it — port forwarding is part of a profile. An opcode 0 is sent first: if the gateway does not answer, no port is shown.
