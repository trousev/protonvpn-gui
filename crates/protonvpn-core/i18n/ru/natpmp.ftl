# The NAT-PMP client that maintains the port-forwarding lease — sanctioned exception #2
# (docs/architecture.md §10.1). Every sentence here is a line in the console note that records the
# lease attempt, or the reason on the Overview page's port-forwarding row, so each one has a single
# line and must stay plain. RFC 6886's own names — NAT-PMP, its opcodes, its result codes — are
# protocol facts and are never translated.

# The socket could not be opened, bound or connected. $detail is the operating system's own
# complaint; it is data and is never translated.
natpmp-io = ошибка сокета: { $detail }

# The gateway never answered within the retry ladder (three tries, ~1.75 s). This is the honest
# "port forwarding is unavailable" answer — never a port number we guessed. $gateway is an
# address:port pair and is data.
natpmp-timeout = шлюз { $gateway } не ответил на запрос NAT-PMP — проброс порта недоступен

# The gateway answered with a non-zero result code. $code is the RFC's numeric code — data, and it
# stays a number; $message is its meaning, translated by the messages below.
natpmp-refused = шлюз отказал (код { NUMBER($code) }: { $message })

# The gateway's answer is not the fixed-size response RFC 6886 describes. $detail says which part
# of it was wrong, from the messages below.
natpmp-malformed = неожиданный ответ шлюза: { $detail }

# A response shorter than the RFC's fixed size. Both numbers are byte counts — data, and $length is
# a number so that a language can decline the word for "byte" properly.
natpmp-malformed-length = { $length ->
        [one] ответ длиной { $length } байт, ожидалось { NUMBER($expected) }
        [few] ответ длиной { $length } байта, ожидалось { NUMBER($expected) }
        [many] ответ длиной { $length } байт, ожидалось { NUMBER($expected) }
       *[other] ответ длиной { $length } байта, ожидалось { NUMBER($expected) }
    }

# The response carries a protocol version this client does not speak. $version is the number the
# gateway sent.
natpmp-malformed-version = версия протокола { NUMBER($version) }

# The response flag or opcode does not match the request. $opcode is the byte that arrived; an
# opcode is a protocol number and is never translated.
natpmp-malformed-opcode = опкод { NUMBER($opcode) }

# The same, when we know what should have arrived. $expected is the opcode that was asked for.
natpmp-malformed-opcode-wanted = опкод { NUMBER($opcode) } вместо { NUMBER($expected) }

# The five meanings of RFC 6886 §3.5's result codes, plus the fallback. Each is read inside
# `the gateway refused (code N: …)`, so each is a noun phrase and never a sentence of its own.
# Result code 0 is success and can never reach this table — the decoder only asks when the code is
# non-zero — but the table is the RFC's, and a mapping with a hole in it invites a wrong answer.

# Result code 0: success.
natpmp-result-ok = успех

# Result code 1: the gateway does not speak this version of the protocol.
natpmp-result-version = версия протокола не поддерживается

# Result code 2: refused by policy. Measured: this is what a non-P2P server, or an account without
# port forwarding, answers.
natpmp-result-not-authorized = не разрешено (нужен P2P-сервер и платный план)

# Result code 3: the gateway could not reach the network.
natpmp-result-network = сетевая ошибка

# Result code 4: the gateway has no mappings left to give.
natpmp-result-resources = у шлюза кончились ресурсы

# Result code 5: the opcode itself is not supported.
natpmp-result-unsupported = операция не поддерживается

# Any other code. The number is printed beside this phrase by `natpmp-refused`, so it is not
# repeated here.
natpmp-result-unknown = неизвестный код
