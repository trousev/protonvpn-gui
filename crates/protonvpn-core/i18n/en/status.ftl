# The connection status as one word. Drawn on the sidebar rail and in the status chip on the
# Overview page. This is what is shown when the CLI has not answered yet: it is not an error and it
# is not "disconnected" either — never invent a status (docs/architecture.md §5).
status-unknown = Unknown

# The connection status as one word, when the CLI says the tunnel is down.
status-disconnected = Disconnected

# The connection status as one word, between asking for a connection and the CLI answering.
status-connecting = Connecting

# The connection status as one word, when the CLI reports a live tunnel.
status-connected = Connected

# The connection status as one word, after the CLI refused or failed. The CLI's own message is shown
# beside it, verbatim.
status-error = Error

# How old a piece of state is (docs/architecture.md §7). It is drawn next to the status it belongs
# to, and it is never the word "stale": the number is the whole message, and the user judges it.
status-age-just-now = updated just now

# The age of state younger than a minute. $seconds is a whole number of seconds, 10 to 59.
status-age-seconds = updated { NUMBER($seconds) }s ago

# The age of state younger than an hour. $minutes is a whole number of minutes, 1 to 59.
status-age-minutes = { $minutes ->
        [one] updated { $minutes } min ago
       *[other] updated { $minutes } mins ago
    }

# The age of state at least an hour old. $hours is a whole number of hours.
status-age-hours = { $hours ->
        [one] updated { $hours } hour ago
       *[other] updated { $hours } hours ago
    }

# The collapsed console bar while the runner has nothing to do (docs/architecture.md §4).
status-runner-idle = idle

# The collapsed console bar while a command runs. $command is the argv, verbatim — the exact program
# and flags that were executed — and is never translated.
status-runner-running = working: { $command }

# The collapsed console bar when commands are waiting behind the running one. They really are
# serialized: two `protonvpn` processes fight over the same state. $depth is how many are waiting.
status-runner-queued = queued: { NUMBER($depth) }

# What a connect is aimed at when nothing particular was asked for. Shown on the Connect button and
# in the console's own label for the command.
status-target-fastest = fastest

# What a connect is aimed at when a server was explicitly asked for at random.
status-target-random = random

# What a connect is aimed at on a Secure Core route. Proton's own product name — leave it as it is.
status-target-secure-core = Secure Core

# What a connect is aimed at on a Tor route. Proton's own product name — leave it as it is.
status-target-tor = Tor

# What a connect is aimed at on a P2P server. It is the CLI's own flag name — leave it as it is.
status-target-p2p = P2P
