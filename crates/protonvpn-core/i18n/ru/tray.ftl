# The tray menu's first item: the connection status as one phrase. The item is disabled — it is a
# label, not a command — and sits above the age line. $status is the status word from status.ftl,
# already in this language; it is never empty, because "Unknown" is its own answer
# (docs/architecture.md §5).
tray-status = Статус: { $status }

# The tray menu item that asks for a connection. Disabled while the CLI reports one. This is the
# CLI's own verb, and the label is short: four items share the menu.
tray-connect = Подключиться

# The tray menu item that takes the tunnel down. Disabled while there is nothing to take down.
tray-disconnect = Отключиться

# The tray menu item that brings the window back — the same thing a click on the icon does. With no
# window open it is the only way in.
tray-show-window = Открыть окно

# The last item of the tray menu. "Quit" is also what the no-tray notice in chrome.ftl points at.
tray-quit = Выход

# The tray menu item for a release that is published but not downloaded yet. $version is the version
# number read out of the release asset's own name — data, never translated.
tray-update-available = Обновление { $version }

# The tray menu item for an image that has been downloaded, verified and swapped in, and is waiting
# for the next start (docs/architecture.md §14). $version is a version number — data, never
# translated. Keep it to one menu line.
tray-update-applied = Обновление { $version } — перезапустите

# The one line this program writes to stderr when the tray cannot be created, which is the case on a
# desktop with no StatusNotifierItem host (GNOME without the AppIndicator extension). $reason is
# ksni's own error text — data, shown verbatim and never translated.
tray-unavailable = protonvpn-gui: системный трей недоступен: { $reason }
