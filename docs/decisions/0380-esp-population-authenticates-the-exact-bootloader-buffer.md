# D-380 ESP population authenticates the exact bootloader buffer

Status: Accepted, 2026-10-10. Closes PW-589.

**Decision.** The one ESP consumer reads `BOOTX64.EFI` once through
D-379's bounded direct-file boundary, reads the adjacent `CHECKSUMS` through
the existing bounded no-follow listing reader, and requires that listing to
contain exactly one entry naming the consumed payload. It hashes the already
read buffer, compares it to that entry, and writes that same buffer to the FAT
file only after the comparison succeeds. Every public or private image route
therefore authenticates at consumption; no optional dispatch-stage verifier and
no second payload open sits between authentication and embedding.

With the reviewed checksum unchanged, a fixed-path substituted bootloader makes
population fail before the destination EFI file exists, and matching bytes
populate and read back exactly. The checksum consumer refuses a redirected
listing, multiple entries, a name other than the consumed adjacent payload, and
a payload path outside the listing's directory. Mutation 1,770 removes only the
consumption-time comparison and fails exactly
`image_population_refuses_a_bootloader_that_misses_its_pinned_digest`, while
D-379's independent path and size mutation remains live.

**Rationale.** The vendor gate verified the `CHECKSUMS` pin, but image, run,
boot and VMware commands could be invoked directly and embedded the checkout
bytes without consulting it, so a substituted EFI payload controlled the image.
Calling `verify-vendor` from each task arm would duplicate policy, miss future
entrypoints and reopen the payload; verifying once before dispatch keeps the
substitution window; and a second digest constant in `xtask` would create two
pins that can drift.
