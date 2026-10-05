# The AppImage updater — exception #4 (docs/architecture.md §14).

# A byte count in binary units, as the download progress reads. $value arrives already rounded to
# one decimal ("4.9", "70.4") or exact for whole bytes, because the arithmetic belongs in Rust and
# only the unit belongs here. The unit symbol itself is translated: Russian writes МиБ, not MiB.
update-bytes-mib = { $value } МиБ

# Bytes in the kibibyte range, same convention. $value is a number already formatted like "812.5".
update-bytes-kib = { $value } КиБ

# Whole bytes, the smallest unit. $value is an exact integer.
update-bytes-b = { $value } Б
