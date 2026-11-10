# D-16 Fully self-contained native-Windows build

Status: Accepted, 2026-07-18.

**Context.** Initial toolchain work assumed WSL2; Isaac rejected it:
"I prefer to keep everything self contained."

**Options.** WSL2 toolchain; native Windows with system installs; native
Windows with everything project-local.

**Decision.** Native Windows, minimal installs, everything else in-repo:
rustup with the **GNU host toolchain** (no Visual Studio; MSVC not
required) and QEMU via winget (an approved system install). Disk imaging is
**pure Rust inside `xtask`**: a hand-rolled MBR partition table and FAT32
ESP via the `fatfs` crate. No WSL, xorriso, mtools or other imaging tools.
`xtask` also writes the VMware artifacts (flat-VMDK descriptor and EFI
`.vmx`) as plain text.

**Rationale.** The kernel is a cross-compiled freestanding ELF, which Rust
builds natively on Windows with its bundled `rust-lld`. Imaging is bytes;
owning it in Rust removes a class of environment fragility and fits the
design-first ethos (we know what is on disk, byte by byte).

**Consequences.** An MBR (not GPT) partition table for now: minimal,
hand-written, and UEFI boots it. GPT arrives with the OS's own partitioning
story. Ask before any new system-level install.
