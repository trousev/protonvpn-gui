# The NAT-PMP client that maintains the port-forwarding lease — sanctioned exception #2
# (docs/architecture.md §10.1). Every sentence here is a line in the console note that records the
# lease attempt, or the reason on the Overview page's port-forwarding row, so each one has a single
# line and must stay plain. RFC 6886's own names — NAT-PMP, its opcodes, its result codes — are
# protocol facts and are never translated.

# The socket could not be opened, bound or connected. $detail is the operating system's own
# complaint; it is data and is never translated.
natpmp-io = socket error: { $detail }

# The gateway never answered within the retry ladder (three tries, ~1.75 s). This is the honest
# "port forwarding is unavailable" answer — never a port number we guessed. $gateway is an
# address:port pair and is data.
natpmp-timeout = the gateway { $gateway } did not answer the NAT-PMP request — port forwarding is unavailable

# The gateway answered with a non-zero result code. $code is the RFC's numeric code — data, and it
# stays a number; $message is its meaning, translated by the messages below.
natpmp-refused = the gateway refused (code { NUMBER($code) }: { $message })

# The gateway's answer is not the fixed-size response RFC 6886 describes. $detail says which part
# of it was wrong, from the messages below.
natpmp-malformed = unexpected answer from the gateway: { $detail }

# A response shorter than the RFC's fixed size. Both numbers are byte counts — data, and $length is
# a number so that a language can decline the word for "byte" properly.
natpmp-malformed-length = { $length ->
        [one] the answer is { $length } byte long, { NUMBER($expected) } expected
       *[other] the answer is { $length } bytes long, { NUMBER($expected) } expected
    }

# The response carries a protocol version this client does not speak. $version is the number the
# gateway sent.
natpmp-malformed-version = protocol version { NUMBER($version) }

# The response flag or opcode does not match the request. $opcode is the byte that arrived; an
# opcode is a protocol number and is never translated.
natpmp-malformed-opcode = opcode { NUMBER($opcode) }

# The same, when we know what should have arrived. $expected is the opcode that was asked for.
natpmp-malformed-opcode-wanted = opcode { NUMBER($opcode) } instead of { NUMBER($expected) }

# The five meanings of RFC 6886 §3.5's result codes, plus the fallback. Each is read inside
# `the gateway refused (code N: …)`, so each is a noun phrase and never a sentence of its own.
# Result code 0 is success and can never reach this table — the decoder only asks when the code is
# non-zero — but the table is the RFC's, and a mapping with a hole in it invites a wrong answer.

# Result code 0: success.
natpmp-result-ok = success

# Result code 1: the gateway does not speak this version of the protocol.
natpmp-result-version = the protocol version is not supported

# Result code 2: refused by policy. Measured: this is what a non-P2P server, or an account without
# port forwarding, answers.
natpmp-result-not-authorized = not allowed (needs a P2P server and a paid plan)

# Result code 3: the gateway could not reach the network.
natpmp-result-network = network error

# Result code 4: the gateway has no mappings left to give.
natpmp-result-resources = the gateway is out of resources

# Result code 5: the opcode itself is not supported.
natpmp-result-unsupported = the operation is not supported

# Any other code. The number is printed beside this phrase by `natpmp-refused`, so it is not
# repeated here.
natpmp-result-unknown = unknown code
