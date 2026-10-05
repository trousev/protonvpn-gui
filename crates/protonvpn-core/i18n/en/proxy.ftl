# Why the local SOCKS5 proxy is not relaying anything — exception #3 (docs/architecture.md §13).
# Shown on the Settings → Proxy card as `closed · <this>`, and repeated verbatim in the console note
# that records the moment the gate shut. It is an explanation, not an error: a proxy that cannot
# show it is protecting you must not claim it is.
proxy-gate-disabled = proxy is off

# The gate is shut because the CLI does not report a connection at all.
proxy-gate-not-connected = VPN is not connected

# The gate is shut because the kernel's route was never seen before the connection: there is
# nothing to compare it against, so nothing is proven. $candidate is an IPv4 address.
proxy-gate-unverified = route { $candidate } is unproven: it was not observed before connecting

# The kernel's route is not the one the tunnel was proven on. Both are IPv4 addresses.
proxy-gate-route-changed = route changed: was { $expected }, now { $observed }

# The proven route is simply gone. $expected is an IPv4 address.
proxy-gate-route-gone = route { $expected } disappeared

# The kernel refused to answer the route question. $detail is the kernel's own complaint.
proxy-gate-route-lost = route lost: { $detail }

# The egress check reports the address we had before connecting, so the tunnel is not carrying
# traffic whatever the CLI says. $ip is an IPv4 address.
proxy-gate-egress-baseline = the egress address is { $ip } again — the same as before connecting: the tunnel is not carrying traffic

# The proxy is on and proven, but there is no listener: the address is not loopback, or the port is
# taken. $detail says which.
proxy-gate-not-listening = proxy is not listening: { $detail }

# The background tunnel check stopped answering. Deliberately not "the route is lost": a check that
# did not answer is not evidence of anything. $detail says what went wrong.
proxy-gate-probe-unanswered = the egress check is silent: { $detail }
