# The connection status as one word. Drawn on the sidebar rail and in the status chip on the
# Overview page. This is what is shown when the CLI has not answered yet: it is not an error and it
# is not "disconnected" either — never invent a status (docs/architecture.md §5).
status-unknown = Неизвестно

# The connection status as one word, when the CLI says the tunnel is down.
status-disconnected = Отключено

# The connection status as one word, between asking for a connection and the CLI answering.
status-connecting = Подключаюсь

# The connection status as one word, when the CLI reports a live tunnel.
status-connected = Подключено

# The connection status as one word, after the CLI refused or failed. The CLI's own message is shown
# beside it, verbatim.
status-error = Ошибка

# How old a piece of state is (docs/architecture.md §7). It is drawn next to the status it belongs
# to, and it is never the word "stale": the number is the whole message, and the user judges it.
status-age-just-now = обновлено только что

# The age of state younger than a minute. $seconds is a whole number of seconds, 10 to 59.
status-age-seconds = обновлено { NUMBER($seconds) } с назад

# The age of state younger than an hour. $minutes is a whole number of minutes, 1 to 59.
status-age-minutes = { $minutes ->
        [one] обновлено { $minutes } минуту назад
        [few] обновлено { $minutes } минуты назад
        [many] обновлено { $minutes } минут назад
       *[other] обновлено { $minutes } минут назад
    }

# The age of state at least an hour old. $hours is a whole number of hours.
status-age-hours = { $hours ->
        [one] обновлено { $hours } час назад
        [few] обновлено { $hours } часа назад
        [many] обновлено { $hours } часов назад
       *[other] обновлено { $hours } часов назад
    }

# The collapsed console bar while the runner has nothing to do (docs/architecture.md §4).
status-runner-idle = жду

# The collapsed console bar while a command runs. $command is the argv, verbatim — the exact program
# and flags that were executed — and is never translated.
status-runner-running = работаю: { $command }

# The collapsed console bar when commands are waiting behind the running one. They really are
# serialized: two `protonvpn` processes fight over the same state. $depth is how many are waiting.
status-runner-queued = в очереди: { NUMBER($depth) }

# What a connect is aimed at when nothing particular was asked for. Shown on the Connect button and
# in the console's own label for the command.
status-target-fastest = быстрейший

# What a connect is aimed at when a server was explicitly asked for at random.
status-target-random = случайный

# What a connect is aimed at on a Secure Core route. Proton's own product name — leave it as it is.
status-target-secure-core = Secure Core

# What a connect is aimed at on a Tor route. Proton's own product name — leave it as it is.
status-target-tor = Tor

# What a connect is aimed at on a P2P server. It is the CLI's own flag name — leave it as it is.
status-target-p2p = P2P
