# What an intent is called in one short phrase. These are labels for a *command we are about to
# run* — shown on a button, in a queue, in a log line — and they never claim an outcome: "connect:
# NL" is what was asked for, not what happened. The launcher maps intents to argv and interprets
# nothing (docs/architecture.md §6), and neither do its labels.
launcher-connect = подключение: { $target }

# The label of a disconnect command.
launcher-disconnect = отключение

# The label of a status refresh.
launcher-status = статус

# The label of `protonvpn countries list`.
launcher-list-countries = список стран

# The label of `protonvpn cities list <CC>`. $country is a two-letter code as the CLI prints it.
launcher-list-cities = города: { $country }

# The label of `protonvpn config list`.
launcher-list-settings = настройки

# The label of `protonvpn info`.
launcher-account = аккаунт

# The label of a sign-in. $username is the account name, which is not a secret; the password and
# the 2FA code never reach a label, a log line or the console.
launcher-sign-in = вход: { $username }

# The label of a sign-out.
launcher-sign-out = выход
