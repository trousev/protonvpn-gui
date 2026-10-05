# Messages for the parts of `protonvpn-core` that are not the engine: the runner, the kernel's
# route answer, the saved-connection summary, and the interpreter's own remarks. They all end up in
# the console, in the collapsed console bar, or in a list row — never in a dialog, so they have one
# line and no room to grow.

# A console line when the child could not be started at all — the CLI is missing, or the PTY could
# not be opened. It is our own sentence, not the CLI's output, and it is the only explanation the
# user gets for a command that produced nothing. $command is the argv, verbatim, and $detail is the
# operating system's own complaint; neither is translated.
core-runner-spawn-failed = не удалось запустить `{ $command }`: { $detail }

# A console line when the user gave up on the running command and it was killed. The line is
# prefixed with `[protonvpn-gui]` by the code, which is how the transcript marks our own words.
core-runner-cancelled = выполнение прервано по запросу пользователя

# A console line when a command outlived its timeout and was killed. $seconds is a whole number of
# seconds — 120 for most commands. The second half matters: the output above the line is everything
# the CLI managed to say before it was stopped, and it must not be read as a complete answer.
core-runner-timed-out = { $seconds ->
        [one] команда не завершилась за { $seconds } секунду и была прервана; вывод выше — всё, что успел сказать CLI
        [few] команда не завершилась за { $seconds } секунды и была прервана; вывод выше — всё, что успел сказать CLI
        [many] команда не завершилась за { $seconds } секунд и была прервана; вывод выше — всё, что успел сказать CLI
       *[other] команда не завершилась за { $seconds } секунды и была прервана; вывод выше — всё, что успел сказать CLI
    }

# The kernel would not answer the proxy's route question at all — the machine is offline, or there
# is no route to the outside. It appears as the reason on the Settings → Proxy card and as the
# detail of the console note that closed the gate, so it is short and never speculates.
core-route-unreachable = нет маршрута наружу

# The same failure with the kernel's own complaint attached. $detail is the operating system's
# error text and is never translated.
core-route-unreachable-detail = нет маршрута наружу: { $detail }

# The kernel answered the route question with an address that cannot be a source address (an
# unspecified, loopback, broadcast or multicast address). $address is an IP address — data.
core-route-unusable = ядро не выбрало адрес источника: { $address }

# A saved connection that names a country but no city, as the second line of its row in the
# connection list. Short: it sits under the name, in a narrow column, beside a city name when there
# is one.
core-summary-any-city = любой город

# A saved connection that names no country at all, in the same place and with the same room.
core-summary-any-country = любая страна

# A badge on a saved connection that keeps a port-forwarding lease (exception #2). It is drawn
# beside the P2P, Tor and Secure Core badges, which are Proton's own names and are not translated;
# this one is our word for a feature that is ours, so it is. Keep it to one short word.
core-badge-port = ПОРТ

# The connection status after a `connect` that exited non-zero and printed no error line of its
# own. It stands in the status chip where the CLI's message would have been, so it has to say both
# which command failed and with what. $command is the argv, verbatim, and $code is the exit code —
# both are data and neither is translated.
core-connect-exit-code = `{ $command }` завершилась с кодом { NUMBER($code) }

# The three ways opening the terminal for a command can fail. None of them is ever a line of its
# own: each is the `$detail` of the runner's "could not start `…`" line, which is the only place a
# PTY failure is shown. $detail is the operating system's or the PTY library's own complaint and is
# never translated — which is also why "pty" here is a technical term, not a word to soften.

# The pseudo-terminal itself could not be opened.
core-pty-open = не удалось открыть псевдотерминал: { $detail }

# The terminal is there but the child could not be started on it. An empty argv is the one case
# with no operating system error behind it.
core-pty-spawn = не удалось запустить процесс: { $detail }

# Reading from or writing to the terminal failed once the child was running.
core-pty-io = ошибка ввода-вывода псевдотерминала: { $detail }

# Reading or writing our own config file failed: the file is corrupt, or the directory is not
# writable. It is shown in the window's notice bar at startup, and again as a console note when a
# save fails while the application is running. $detail is the operating system's or the JSON
# parser's own complaint and is never translated; the path is deliberately not part of the
# sentence, because the notice already belongs to this application.

# The file could not be read or written at all.
core-config-io = не удалось прочитать или записать файл настроек: { $detail }

# The file was read and is not valid JSON for this version of the application.
core-config-parse = не удалось разобрать файл настроек: { $detail }
