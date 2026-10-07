# The AppImage updater — exception #4 (docs/architecture.md §14).

# A byte count in binary units, as the download progress reads. $value arrives already rounded to
# one decimal ("4.9", "70.4") or exact for whole bytes, because the arithmetic belongs in Rust and
# only the unit belongs here. The unit symbol itself is translated: Russian writes МиБ, not MiB.
update-bytes-mib = { $value } MiB

# Bytes in the kibibyte range, same convention. $value is a number already formatted like "812.5".
update-bytes-kib = { $value } KiB

# Whole bytes, the smallest unit. $value is an exact integer.
update-bytes-b = { $value } B

# Everything below is a failure the updater reports about itself. Each one is a single line: it is
# written under the `curl …` invocation in the console, and the same sentence is the reason on the
# Settings → Updates card. Paths, URLs, versions, checksums, file names and the operating system's
# error text are data and are never translated.

# `FromStr` for a release version was handed something that is not `X.Y.N`. $text is the string as
# it arrived, verbatim.
update-error-version-parse = not a version of the form X.Y.N: { $text }

# `curl` is not installed, or could not be run at all. $detail is the operating system's complaint.
update-error-curl-missing = curl is not available: { $detail }

# `curl` ran and failed — no route, a timeout, a 404. $code is the process exit code, data.
update-error-curl-failed = curl finished with exit code { NUMBER($code) }

# The same failure with `curl`'s own last words attached. $detail is what `curl` wrote to stderr,
# verbatim.
update-error-curl-failed-detail = curl finished with exit code { NUMBER($code) }: { $detail }

# The release page answered and there is nothing in it this build could become. $sums is the file
# that was read (`SHA256SUMS`) and $wanted is the name shape that was looked for, spelled out with
# the placeholders a name has — both are data and neither is translated.
update-error-no-asset = nothing to install: { $sums } has no line naming { $wanted }

# The downloaded bytes do not hash to the checksum the release published. Both values are 64 hex
# characters copied from the two documents, and neither is translated or abbreviated.
update-error-checksum = the checksum does not match: expected { $expected }, got { $got }

# The bytes hash correctly and are still not a type-2 AppImage. $bytes is the first handful of bytes
# rendered for reading, escape sequences and all — data, shown so the console can be believed.
update-error-not-appimage = what was downloaded does not look like an AppImage: first bytes { $bytes }

# There is nowhere to put the new image, so downloading it would be pointless. $detail is a path
# followed by the operating system's complaint.
update-error-not-writable = cannot write the new image beside the installed one: { $detail }

# The user cancelled the transfer.
update-error-cancelled = the download was cancelled

# A read, a write or a rename failed. $detail is the operating system's own complaint.
update-error-io = input/output error: { $detail }
