# D-16 Fully self-contained native-Windows build

Status: Accepted, 2026-07-18.

**Decision.** Native Windows, minimal installs, everything else in-repo:
rustup with the **GNU host toolchain** (no Visual Studio; MSVC not required)
and QEMU via winget (an approved system install). No WSL. Disk imaging is
**pure Rust inside `xtask`**: a hand-rolled MBR partition table and FAT32
ESP via the `fatfs` crate, with no xorriso, mtools or other imaging tools.
`xtask` also writes the VMware artifacts (flat-VMDK descriptor and EFI
`.vmx`) as plain text.

**Rationale.** Isaac wants everything self-contained. Rust builds the
freestanding kernel natively on Windows with its bundled `rust-lld`, and
owning the imaging bytes removes a class of environment fragility.

**Consequences.** An MBR (not GPT) partition table for now; GPT arrives with
the OS's own partitioning story. Ask before any new system-level install.
