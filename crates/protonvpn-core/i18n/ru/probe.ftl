# The ground-truth egress probe — sanctioned exception #1 (docs/architecture.md §0, §8). `curl` asks
# an echo service what address it sees, and these are the three ways that can fail. Each one is a
# single line under the `curl …` invocation in the console, so there is room for one sentence and
# no more. None of them says anything about the tunnel: a check that did not answer is not evidence
# about the route, and the wording must not imply that it is.

# `curl` is not installed, or could not be run at all. $detail is the operating system's own
# complaint and is never translated.
probe-curl-missing = curl недоступен: { $detail }

# `curl` ran and failed — no route, a timeout, or a server that does not answer IPv6. $code is the
# process exit code; the code is data and so is the `curl` name.
probe-curl-failed = curl завершился с кодом { NUMBER($code) }

# The same failure with `curl`'s own last words attached. $detail is whatever `curl` wrote to
# stderr, verbatim — never translated, never reworded.
probe-curl-failed-detail = curl завершился с кодом { NUMBER($code) }: { $detail }

# `curl` succeeded and the body is not the JSON this probe knows how to read — an error page behind
# a captive portal, or a service that changed its field names. $body is the response body as it
# arrived, truncated by the caller if it is long.
probe-unparseable = не удалось разобрать ответ: { $body }
