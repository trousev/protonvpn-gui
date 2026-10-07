# The heading of the connection editor — the modal that covers whichever page is open. The widget
# draws it in capitals itself, so write it in ordinary case. This is the variant for a new
# connection; the modal is 760px wide and the heading shares its row with the close button.
editor-title-new = New connection

# The same heading when an existing saved connection is being changed. The connection keeps its id
# through a rename, so this is an edit of the thing that is already selected.
editor-title-edit = Edit connection

# The line under the heading, 12pt and muted. The country and city lists are not ours: they are what
# `protonvpn countries list` and `protonvpn cities list` print. Both command names are data and are
# never translated.
editor-subtitle = The country and the city come from protonvpn countries list and protonvpn cities list

# The label above the connection's name field, 13pt. The name itself is the user's own text, not
# ours — this is only the caption.
editor-name-label = Connection name

# The placeholder inside the empty name field: an example of the kind of name a person gives a saved
# connection, not a label. Translate the example so that it reads as a plausible name in the target
# language rather than as a translation of this one.
editor-name-placeholder = for example, Work

# The placeholder inside the search field above the country list. It filters the list the CLI
# printed, by country name or by two-letter code. Not a label: it disappears when typing starts.
editor-search-placeholder = Search for a country or a code

# The first row of the country list: the option that leaves the country unset, so a connect aims at
# the fastest server instead of a country. It is a real choice, drawn exactly like every country
# below it, not a placeholder. The hint under it — editor-any-country-hint — says what that means
# for the command that will run.
editor-any-country = Any country

# The small grey line under editor-any-country. `--country` is the CLI's own flag: never translate
# it, and keep it exactly as written, dashes included.
editor-any-country-hint = without --country

# The note that stands in for the country list until `protonvpn countries list` has answered. That
# command runs once per start, so this is what the panel shows while it is still in flight. The
# command name is data and is never translated.
editor-countries-pending = The list of countries has not been read yet: protonvpn countries list runs once per start.

# The note shown under that one when the CLI refused to list countries and answered with an error.
# $message is the CLI's own error text, verbatim — the exact bytes it printed. Never translate,
# shorten or reword it; only the words around it are ours.
editor-country-error = The CLI answered with an error: { $message }

# Shown in the country list when the search box filters every country out. It is a statement about
# the filter, not about the CLI: the list itself may be complete. One short faint line.
editor-country-empty = Nothing found.

# The heading of the right-hand panel while no country has been picked. The widget draws it in
# capitals itself, so write it in ordinary case. The panel cannot show cities for a country that has
# not been chosen.
editor-cities-title = Cities

# The muted line under that heading, saying why the panel is empty and what to do about it. "cities
# are requested for one" is literal: the CLI's city list takes a country.
editor-cities-pick-country = Pick a country first — cities are requested for one.

# The heading of the city panel once a country is chosen. $country is the country's name exactly as
# the CLI printed it — data, never translated — and the widget draws the whole line in capitals.
editor-cities-title-for = Cities · { $country }

# The muted line shown in the city panel while the city list for the chosen country is still being
# read. No command name here: the line is short, and the console underneath already shows what is
# running.
editor-cities-loading = Requesting the city list…

# The first row of the city list: the option that leaves the city unset, so the CLI picks the fastest
# server in the chosen country. A real choice, like editor-any-country.
editor-any-city = Any city

# The small grey line under editor-any-city, saying what leaving the city unset means.
editor-any-city-hint = the fastest in the country

# Shown in the city panel when the CLI's list for the chosen country holds no cities. It is not an
# error: some countries simply have none in the CLI's data.
editor-cities-empty = There are no cities in this country's list.

# The secondary button at the bottom of the modal: close the editor and change nothing. Short, 13pt,
# in the row beside editor-save.
editor-cancel = Cancel

# The primary button at the bottom of the modal: keep the connection. Short, 13pt. It confirms
# nothing the CLI did — saving writes a profile into our own config, and that is all.
editor-save = Save
