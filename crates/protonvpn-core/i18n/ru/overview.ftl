# The eyebrow above the Overview page's title: the small caption that names the page. It is drawn
# in capitals by the widget, so write it the way it should read and do not capitalise it here.
overview-page-eyebrow = Обзор

# The Overview page's title, right under that eyebrow. It is the largest text on the page and the
# page's own subject, not the tunnel's state: the state is the card below it.
overview-page-title = Соединение

# The button at the top right of the Overview page. It asks the CLI for a fresh `status`; it does
# not connect anything. Short: it shares its row with the page title.
overview-refresh-status = Обновить статус

# The big line in the status card when the CLI reports a live tunnel. It is the server and where it
# is, as the CLI printed them — `NL#818 in Amsterdam, Netherlands`.
# $server is a server name and $location is the CLI's own location string; both are data and
# neither is translated.
overview-status-connected = { $server } в { $location }

# The big line in the status card while a connect is in flight. It claims the intent only: the CLI
# has not answered yet, so nothing about the tunnel is known (docs/architecture.md §5).
overview-status-connecting = Подключаюсь…

# The big line in the status card when the CLI says there is no tunnel.
overview-status-disconnected = Нет активного туннеля

# The big line in the status card after the CLI refused or failed. The CLI's own message is shown
# verbatim in the note underneath, so this line says only whose answer it was.
overview-status-error = CLI отказал

# The big line in the status card before the CLI has answered anything at all. This is not an error
# and it is not "disconnected": nothing is known yet.
overview-status-unknown = Состояние неизвестно

# The note under that line, and the reason it exists: an unknown state is shown as unknown instead
# of being guessed at (docs/architecture.md §5).
overview-status-unknown-note = CLI ещё не отвечал — состояние не выдумывается.

# The status card's main button while a tunnel is up. It tears the tunnel down.
overview-disconnect = Отключиться

# The status card's main button when there is no tunnel. It connects the selected connection: a
# request to the CLI, not a promise about the outcome.
overview-connect = Подключиться

# One of the four small tiles under the status line. The widget draws the label in capitals and
# puts a value beside it; the value is data (a server name, a city, a percentage, a protocol) and is
# never translated. "Server" means the CLI's own server identity, not this application.
overview-tile-server = Сервер

# The second tile: the city half of the CLI's location string. Data beside it.
overview-tile-city = Город

# The third tile: the server's load in percent, or an em dash when the CLI did not report one.
overview-tile-load = Нагрузка

# The fourth tile: the protocol the CLI reported, spelled the CLI's way (data).
overview-tile-protocol = Протокол

# The line under the status title: how old this state is, and which connection the Connect button
# is aimed at. $age is already a translated phrase ("updated 3 mins ago"); $target is the selected
# connection's name — one of the CLI's presets or a profile the user named — and is data.
overview-age-and-target = { $age } · соединение: { $target }

# The eyebrow of the egress probe card. An eyebrow: drawn in capitals by the widget.
# "egress" is the address traffic leaves by; "ground truth" is deliberate — a measurement is the
# evidence, and the CLI's own report is not (docs/cli-surface.md §4.9).
overview-egress-eyebrow = Проба egress · ground truth

# The egress card's button: run the probe now.
overview-egress-measure = Измерить

# A fact row in the egress card: the address the service sees right now. The value beside the
# label is an IP address — data, never translated. There is deliberately no "before" row beside
# it: the application keeps one reading and compares nothing.
overview-egress-ip = Текущий IP

# A fact row in the egress card: the country the GeoIP service reported for that address.
# "(advisory)" is the honesty — the geo databases disagree with each other, so this is not
# evidence of anything.
overview-egress-country = Текущая страна (справочно)

# A fact row in the egress card: the autonomous system and organisation the service reported —
# the network traffic leaves by. The value is data; "ASN" is the standard abbreviation and stays
# as it is.
overview-egress-asn = Провайдер (ASN / организация)

# A fact row in the egress card: which service answered the probe. The value is data (a host name).
overview-egress-source = Источник

# The note under the egress card's rows: when the reading is taken, and the one thing this card
# deliberately does not do — compare. Whether the address moved is for the reader to notice.
overview-egress-current-note = Берётся при запуске и при каждом подключении и отключении; «Измерить» берёт её снова. Здесь ничего не сравнивается: изменился адрес или нет — видеть вам, а не судить нам.

# The note under those rows before anything has been measured at all: the external check is off in
# the settings, or no service has answered yet. `Measure` is the button beside the eyebrow.
overview-egress-none = Пока не измерено. Нажмите «Измерить» — а если проверка выключена в настройках, сначала включите её.

# The faint paragraph at the bottom of the egress card: why this card may look redundant beside the
# status. It is a design decision written out, and it stays blunt — the CLI can be wrong about the
# egress address, so the measurement outranks it.
# "ifconfig.co/json" is a URL and "protonvpn" is the program we run: both are data, not words.
overview-egress-provenance = Проба к ifconfig.co/json — единственный внешний вызов помимо protonvpn. CLI может ошибаться в адресе выхода, поэтому источник истины — измерение, а не самоотчёт. Страна и ASN — только для чтения: базы GeoIP расходятся между собой.

# The eyebrow of the connections card, which lists the CLI's presets and the user's profiles. An
# eyebrow: drawn in capitals.
overview-connections-eyebrow = Соединения

# The button in that card's header, at the top right: it opens the profile editor on a new profile.
overview-connections-add = Добавить соединение

# The faint caption above the three presets. They are the CLI's own shortcuts and cannot be
# renamed, edited or deleted — this line is why they have no Edit and Delete buttons.
overview-connections-system = Системные · не редактируются

# The faint caption above the user's own profiles, below the presets.
overview-connections-mine = Мои соединения

# Shown in place of the profile list when the user has none. "Add connection" is the name of the
# button above it, written out because a message may not refer to another message.
overview-connections-empty = Пока нет своих соединений. «Добавить соединение» соберёт профиль: страна, город, P2P, Secure Core, Tor и проброс порта.

# The faint line at the bottom of the connections card: whatever is selected, the command is always
# a connect for it.
# "protonvpn connect" is the argv of the command we will run — data, never translated.
overview-connections-footer = Подключение всегда выполняется для выбранного соединения: protonvpn connect.

# The rename-and-edit button on one of the user's own profile rows. Not a preset row: those have no
# such button. Short, and one of a pair with Delete beside it.
overview-connection-edit = Изменить

# The delete button on one of the user's own profile rows, beside Edit. Destructive and short; the
# row it belongs to is already on screen, so the label does not repeat the profile's name.
overview-connection-delete = Удалить

# The port-forwarding card's header label, at the right end of the row that carries the eyebrow:
# port forwarding is off for the selected connection, so there is nothing to show. Lowercase, one
# line, narrow — the eyebrow on its left is long.
overview-port-off = выключен для этого соединения

# The same header label while a NAT-PMP lease is held.
overview-port-active = аренда активна

# The same header label while a lease has been asked for and the gateway has not answered yet.
overview-port-pending = запрашиваю аренду

# The same header label when the connected server cannot forward a port at all. "it" is port
# forwarding, not the server; the full sentence is a note below, not here.
overview-port-unsupported = сервер не поддерживает

# The same header label when the lease failed or cannot be held.
overview-port-unavailable = недоступен

# The same header label when the card is on but no lease has been asked for yet.
overview-port-idle = аренды нет

# The eyebrow of the port-forwarding card. An eyebrow: drawn in capitals. "Port forwarding" is the
# feature; the card is about the NAT-PMP lease that provides it.
overview-port-eyebrow = Порт-форвардинг

# The button beside the port number that copies it to the clipboard. Short: the number itself is
# drawn large next to it.
overview-port-copy = Копировать

# The line under the port number: how long the current lease lasts and that it is renewed without
# being asked. $seconds is the lease lifetime in whole seconds — a number, not data to translate.
overview-port-lease = аренда { NUMBER($seconds) } с, продлевается автоматически

# Shown for a couple of seconds after the copy button is pressed, in place of the line below.
overview-port-copied = Порт скопирован в буфер обмена

# The line under the port number while a lease is held, after the copy confirmation has faded: the
# port is not stable, and the user is about to paste it somewhere.
overview-port-volatile = Порт выдаётся шлюзом и меняется после переподключения.

# The note beside the em dash when a lease could not be obtained. It is a design decision and it
# stays this blunt: a number nobody renews would be worse than no number.
overview-port-hidden = Порт не показывается намеренно: показывать номер, который никто не продлевает, значит вводить в заблуждение.

# The line under the em dash while the gateway is being asked, after the request was sent.
# "NAT-PMP" is a protocol name and stays as it is.
overview-port-requesting = Запрашиваю аренду у шлюза через NAT-PMP…

# The line under the em dash when the connected server does not support port forwarding. "P2P" is
# the CLI's own term for the servers that do.
overview-port-unsupported-note = Этот сервер не поддерживает проброс порта. Подключитесь к P2P-серверу.

# The line under the em dash when port forwarding is on for the profile but there is no lease at
# all — nothing has been asked for yet.
overview-port-none = Аренды нет.

# The label of the toggler that decides whether the selected profile asks for a lease. It appears
# only for a profile the user owns; the presets have no such switch.
overview-port-keep = Держать аренду для этого профиля

# Shown instead of that toggler when a system preset is selected: presets are fixed, and a profile
# is how a user gets a setting of their own. "Add connection" is the button's label in the
# connections card, written out because one message may not refer to another.
overview-port-system-preset = Системные пресеты не редактируются: профиль создаётся кнопкой «Добавить соединение».

# The button that asks the gateway for a lease again, used when the automatic request failed.
overview-port-refresh = Запросить заново

# The button that gives the current lease up. It sits beside Request again, in the muted style
# because it is not an emergency.
overview-port-release = Освободить

# The faint paragraph at the bottom of the port-forwarding card: where the number comes from, and
# why a port is sometimes withheld. It is a design decision written out and stays exact.
# "10.2.0.1:5351" is an address, "RFC 6886" is a document number and "opcode 0" is a field value —
# all data, all shown as they are.
overview-port-provenance = Порт выдаётся лизом NAT-PMP у 10.2.0.1:5351 (RFC 6886) только если это включено в выбранном соединении — порт-форвардинг является частью профиля. Перед запросом отправляется opcode 0: если шлюз не отвечает, порт не показывается.
