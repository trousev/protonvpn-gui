# The engine's own words: every note it writes into the console, every sentence it puts in the
# window's note bar, and the port-forwarding state it hands to the Overview page. The engine owns
# the one thread that knows what happened, so these sentences are the product's explanation of its
# own behaviour (docs/architecture.md §1, §10.4). A note is never dressed up as a `protonvpn`
# command, and nothing here may claim more than was observed.

# The note title of a NAT-PMP release, shown as `NAT-PMP release <port> (<this>)`. It is a fragment,
# not a sentence — it says why the lease was given up, in parentheses. Short: one note title line.
engine-lease-why-quit = выход из приложения

# The same, when the connection moved to another server and the old mapping went with it.
engine-lease-why-connection-changed = подключение изменилось

# The same, when the user asked for the port to be released.
engine-lease-why-user-request = по запросу пользователя

# The same, before a connect that will change which server we are on.
engine-lease-why-before-connect = перед новым подключением

# A note in the console when none of the three ground-truth endpoints answered, so the probe cannot
# be used at all. It says the check is unavailable — it never says anything about the tunnel.
engine-probe-unavailable = ни один сервис проверки внешнего адреса не ответил

# A note when the user asked for an egress check with the probe switched off in the settings.
engine-probe-disabled = проверка внешнего адреса отключена в настройках

# Why the connect-at-startup was not issued: the first `status` of the session already reported a
# live tunnel, and `connect` against a live tunnel switches servers silently
# (docs/cli-surface.md §4.4). $server is what the CLI reported, spelled `NL#818 · Amsterdam,
# Netherlands` — data, never translated.
engine-startup-connect-already = уже подключено: { $server }

# The same decision, when a connection was already in flight.
engine-startup-connect-in-progress = подключение уже выполняется

# The title of the note that records the decision above. `connect` is the CLI's own subcommand name
# and stays as it is; the sentence around it is ours.
engine-startup-connect-skipped = connect при старте пропущен

# A note when the command could not even be handed to the runner — the queue is gone, which means
# the application is on its way down. There is no command line to show.
engine-submit-failed = не удалось поставить команду в очередь

# A note when writing a secret to the CLI's terminal failed. Deliberately vague about which secret:
# a password or a 2FA code never appears in the console.
engine-stdin-failed = не удалось передать ввод в процесс CLI

# A note when the window offers an answer but the CLI is not asking anything — a stale prompt.
engine-no-prompt = CLI сейчас не ждёт ввода

# The first line of the port-forwarding note: the address the gateway sees us on, which is what the
# lease is about. $address is an IP address — data.
engine-lease-gateway-address = публичный адрес шлюза: { $address }

# One line of the port-forwarding note per protocol. $protocol is `UDP` or `TCP`, a protocol name
# and never translated; $external and $internal are ports and $lifetime a number of seconds, all
# data. The note records the mapping as it was granted.
engine-lease-mapped = { $protocol }: внешний порт { NUMBER($external) } (внутренний { NUMBER($internal) }), срок { NUMBER($lifetime) } с

# Why the port-forwarding row on the Overview page says the feature is unavailable: the gateway
# answered, and neither protocol was granted a mapping. Short: it is drawn on one row beside the
# state, so it is a clause, not a sentence.
engine-lease-no-port = шлюз не выдал порт

# The same row after a renewal failed. The lease is deliberately dropped at that moment rather than
# shown as if it were still ours (docs/cli-surface.md §4.7).
engine-lease-renew-failed = не удалось продлить аренду порта

# The console note that says the lease is gone, and why. This is the sentence that keeps the console
# trustworthy: a port that cannot be confirmed is not a port.
engine-lease-lost = проброс порта потерян: шлюз не подтвердил продление

# One line of the release note per protocol. $protocol is `UDP` or `TCP` — a protocol name, data.
engine-lease-released = { $protocol }: аренда освобождена

# Why the SOCKS5 proxy has no listener: the address in the settings is not a loopback address. It
# becomes the detail of `proxy is not listening: …` on the Settings → Proxy card and in the console
# note. $address is what the user typed into the address field, verbatim; `127.0.0.1` and
# `localhost` are addresses, not words.
engine-socks5-bad-address = адрес `{ $address }` не подходит: прокси слушает только петлевой IPv4 (127.0.0.1 или localhost)

# The console note that records the proxy coming up. It is a promise about what the listener does
# and nothing more: the gate is what decides, connection by connection.
engine-socks5-listening = слушает: соединения пойдут только через туннель

# The console note that records the listener failing. $detail is one of the `proxy-…` messages
# saying which way it failed.
engine-socks5-not-listening = не слушает: { $detail }

# The console note that records the gate opening: the kernel's source address differs from the one
# observed while the CLI said the tunnel was down. $candidate and $reference are both IPv4
# addresses — data — and the wording must not claim the route *is* the tunnel, only that it
# changed.
engine-socks5-route-proven = маршрут подтверждён: { $candidate } (отличается от наблюдённого { $reference })

# The console note that records the gate shutting while it had been open. $reason is one of the
# `proxy-gate-…` sentences above, and it is the whole explanation — this message only labels it.
engine-socks5-closed = закрыт: { $reason }

# The reason the gate shut after two background checks in a row went unanswered. It is the detail of
# `the egress check is silent: …`; a check that did not answer is not a statement about the route.
engine-socks5-two-probes-silent = две проверки подряд не ответили

# A note when a check was asked for while one is already in flight. One job at a time, and the one
# running is the answer.
engine-update-check-running = проверка обновлений уже идёт

# Why nothing was downloaded: this build did not come from an AppImage, so there is no file it could
# replace. `AppImage` is the packaging format's own name and stays as it is.
engine-update-not-an-appimage = эта сборка не AppImage — подменить себя не могу, обновление придётся скачать вручную

# The same note when the release is not even known yet, because the check has not run.
engine-update-nothing-to-fetch = не знаю, что скачивать: сначала нужна проверка обновлений

# The title of every update note, as `update <version>`. It is a heading, not a sentence; $version
# is a release version such as 0.1.43 — data, never translated.
engine-update-note = обновление { $version }

# A note when the user asked to install and the image is already in place. Nothing is downloaded
# twice: the new image takes effect at the next start, always.
engine-update-already-installed = обновление уже на месте — оно заработает при следующем запуске

# The single line of the note that records a successful check. $version is the release tag — data.
engine-update-latest = последний релиз: { $version }

# The note when this build does not know its own version, so no comparison can be made in either
# direction. A build from `main` between releases is exactly this case.
engine-update-unversioned = эта сборка без версии — сравнивать не с чем

# The note when a release exists and this build cannot install it: a tarball install, or a `cargo
# run`. $version is the release tag — data.
engine-update-available-manual = доступна версия { $version }, но эта сборка не AppImage — скачайте её вручную

# One line of the note that records a finished download. $bytes is already formatted with its unit
# (`4.9 MiB`) — data. The checksum matching is the whole point of the sentence.
engine-update-downloaded = скачано { $bytes }, контрольная сумма совпала

# The second line of the same note: the bytes are verified and waiting, and nothing has been
# replaced yet. $version is the release tag — data.
engine-update-staged = { $version } ждёт установки: заработает после перезапуска

# The first line of the note that records the swap. $version is the release tag — data.
engine-update-installed = образ { $version } занял место предыдущего

# The second line, when the previous image was kept as a hard link. $path is a file path — data, and
# it is the whole point of the line: that is the file to restore if the new image misbehaves.
engine-update-backup = предыдущий оставлен как { $path } и будет удалён при следующем запуске

# The same line when the hard link could not be made: the update went through and there is nothing
# to roll back to. Said plainly rather than left implicit.
engine-update-no-backup = предыдущий сохранить не удалось — откатываться будет нечем

# The last line of the installed note. Nothing was restarted, deliberately: the image on disk is new
# and the process is still the old one (docs/architecture.md §14).
engine-update-running-old = сейчас работает старый образ: новый заработает при следующем запуске

# A note when the user cancelled a download in progress.
engine-update-cancelled = загрузка обновления отменена

# The title line of an update note when the arguments that ran are not known — the release or the
# installed image is missing. It names the program only; `curl` is the program's own name.
engine-update-curl-fallback = curl (обновление)

# The next three are the *titles* of the port-forwarding notes — the line the console shows above
# the note's own lines, where a real invocation would show its argv. They are ours and not the
# CLI's: no program is run for a lease, `natpmpc` is not installed and never called. `NAT-PMP` is
# the protocol's own name and stays as it is; everything around it is a sentence fragment, short
# enough for a collapsed note header.

# Opcode 0, asked first: what address does the gateway see us on? $gateway is an address:port pair.
engine-lease-note-public-address = NAT-PMP: публичный адрес { $gateway }

# One mapping request. $protocols is the protocol name or names asked for — `UDP`, `TCP`, `UDP+TCP`
# — and is data; $gateway is an address:port pair. The note's own lines say what came back, so the
# title must not claim the mapping was granted.
engine-lease-note-map = NAT-PMP: проброс { $protocols } { $gateway }

# Giving the mapping back. $port is the port that was held and $why is the reason from the four
# messages above — the same fragment, in parentheses.
engine-lease-note-release = NAT-PMP: освобождение { NUMBER($port) } ({ $why })
