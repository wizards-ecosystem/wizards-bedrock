# 001 — Boot

## Decision

Boot via **UEFI + Limine** (boot protocol base revision 3, `limine` crate
0.5.0 — pinned because 0.6.x requires nightly). Writing our own bootloader
is deferred to a side-quest; Limine gives us:

- entry directly in 64-bit long mode, higher half
- framebuffer, memory map, HHDM offset, module loading (future initrd)
- identical behavior on QEMU/EDK2, VMware EFI, and real UEFI hardware

## Kernel image contract

- ELF64, linked at `0xffffffff80000000` (linker script `kernel/linker.ld`)
- built for `x86_64-unknown-none` with `code-model=kernel`,
  `relocation-model=static` (`.cargo/config.toml`)
- Limine requests live in a `.requests` section between
  `.requests_start_marker` / `.requests_end_marker` (kept by linker script)
- entry: `extern "C" fn kmain() -> !`

## Disk layout (built by `cargo xtask image`, pure Rust)

64 MiB raw image, MBR partition table (not GPT — deliberately minimal and
hand-written in `xtask`; GPT comes when the OS grows a partitioning story):

- partition 1: type `0xEF` (EFI System), FAT32, from LBA 2048
  - `/EFI/BOOT/BOOTX64.EFI` — Limine v10 (vendored in `boot/limine/`)
  - `/boot/limine/limine.conf` — from `boot/limine.conf`
  - `/boot/wizard-kernel.elf` — the kernel

Limine finds its config at `boot():/boot/limine/limine.conf` (first entry in
its search order) and loads the kernel per `path: boot():/boot/wizard-kernel.elf`.

## Runners

- **QEMU** (`cargo xtask run`): q35 machine, EDK2 firmware from the QEMU
  install (copied into `target/` as pflash code+vars), serial → stdio.
- **VMware** (`cargo xtask vmware`): generates `vm/` containing a flat-VMDK
  descriptor over a copy of the raw image + a `firmware = "efi"` VMX;
  serial → `vm/serial.log`. Start with VMware GUI or
  `vmrun start vm\wizard-os.vmx`.

## Verified

2026-07-18 — both QEMU (1280x800 fb) and VMware (1024x768 fb) boot to the
framebuffer banner with serial log:

```
The Wizard's OS awakens.
limine: base revision supported
framebuffer: WxH bpp=32
kmain: entering idle loop
```

## Debug channels

- COM1 serial (`kernel/src/serial.rs`) — primary log, works everywhere
- framebuffer console (`kernel/src/framebuffer.rs`) — Noto Sans Mono
  bitmap glyphs, early-boot banner only (proper console comes in phase 1)

## Open items

- GDB stub debugging (`-s -S`) — wire a `cargo xtask gdb` once we have a
  debugger story on Windows (gdb isn't installed natively; candidates:
  lldb, gdb-multiarch via a future decision)
- Limine crate 0.6.x when its nightly requirement lands on stable, or if
  we move to nightly for other reasons
