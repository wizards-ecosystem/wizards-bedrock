# D-379 Boot-image sources are bounded at their consuming copy

Status: Accepted, 2026-10-10. Closes PW-588.

**Decision.** Image assembly reads `BOOTX64.EFI` and `limine.conf` through the
existing D-344 checked reader inside the consuming copy operation. Every
ancestor and the immediate parent must remain a real directory, the source must
be one direct regular child, and the opened handle cannot follow a
final-component link. Initial length is checked before allocation, one byte
past the ceiling detects growth, and final length plus direct-file shape are
rechecked. The Limine configuration has a 64 KiB ceiling; the EFI executable
uses the same 64 MiB reviewed vendored-payload ceiling as the vendor verifier.
All image, run, boot-check, degraded and VMware paths inherit the rule through
their one shared ESP population boundary rather than an optional earlier gate.

A focused host witness refuses a fixed-source link to an external sentinel and
a direct file one byte beyond its ceiling before creating the destination
entry, and copies a bounded direct file byte for byte. Mutation 1,769 restores
the eager following `std::fs::read` and fails exactly
`redirected_image_sources_are_refused_before_copy`.

**Rationale.** The ESP population helper had copied both sources with
`std::fs::read`, which followed a final-component link and allocated through end
of file before FAT capacity could bound the destination, so a hostile checkout
could consume host memory or embed bytes outside the reviewed tree.
`verify-vendor` does not cover direct image commands or `limine.conf`, checking
metadata before the old read keeps the link-swap window, and streaming straight
into FAT would need rollback for a source that grows or changes during the
write.
