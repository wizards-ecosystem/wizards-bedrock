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

The requests and the `kmain` symbol are emitted by `wost::entry!(kernel_main)`,
which expands *in the binary crate* — `#[used]` statics inside an rlib can be
dropped by the linker (D-21). The kernel itself sees only `wost::boot::BootInfo`
and never names the bootloader protocol; `wost/src/boot/limine_glue.rs` is the
whole of that dependency.

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
framebuffer banner.

2026-08-12 — re-verified on both after the framekernel split (D-21). QEMU at
1280x800, VMware at 1024x768; the banner is byte-identical to before
(background `0x0B1021`), and the panic path, double-panic guard, and console
scrolling were each exercised. VMware's SVGA-II is the framebuffer that could
have differed from QEMU's under the new mask-driven pixel packing, so its
boot is the one that confirms `libs/pixel` reads the bootloader's channel
masks correctly. Serial log (QEMU):

```
[info ] The Wizard's OS awakens.
[info ] limine: base revision supported
[info ] framebuffer: 1280x800 pitch=5120 bpp=32
[info ] kernel: entering idle loop
```

## Debug channels

- COM1 serial (`kernel/src/arch/x86_64/uart.rs`) — primary log, works
  everywhere. Polled 16550 at 115200 8N1, with a loopback probe at bring-up so
  a missing controller is reported rather than silently swallowing every line.
  Records carry a level tag; `debug!` is filtered out by default.
- framebuffer console (`kernel/src/console.rs`) — Noto Sans Mono bitmap
  glyphs, scrolling, pixel packing driven by the bootloader's channel masks
  rather than an assumed `0xRRGGBB` (`libs/pixel`). Carries user-facing output
  and the panic screen; diagnostics stay on serial.

Both are brought up before the Limine base revision is checked, so a machine
with neither a serial cable nor a supported bootloader still shows why it
stopped. A framebuffer that is not direct RGB, not 32bpp, or not 4-byte aligned
in address and pitch is refused with a typed reason rather than drawn on: the
alternative failure mode is every access being rejected and the screen simply
staying blank.

## Known limitations

Things that work today and will not survive the next phase. Each is a
deliberate choice to keep Phase 1 finishable, not an oversight.

- **`wost::sync::SpinLock` does not touch the interrupt flag.** A lock taken in
  normal kernel context and also taken from an interrupt handler will deadlock.
  Unreachable until the kernel takes interrupts — which is Phase 2. **Fix this
  before the first IRQ handler logs anything**, and fix it in `SpinLock`
  (an `irqsave` acquire), not at the call sites.
- **`Console::scroll` reads the framebuffer back.** Rows move as raw words, so
  nothing is lost, but framebuffer memory is uncached and a full-screen scroll
  is on the order of a million slow reads. Fine for a banner, visibly slow if
  anything ever logs to screen in a loop. A software back-buffer is the answer
  once there is a heap (Phase 3).
- **Logging level is a compile-time constant** (`MAX_LEVEL` in
  `kernel/src/log.rs`). No runtime control and no per-module filtering until
  there is somewhere to configure it from.
- **The kernel image is not relocatable.** `linker.ld` hardcodes
  `0xffffffff80000000`, which is what blocks turning on `kaslr: yes` in
  `boot/limine.conf` (`002-architecture.md`, security baseline).

## Open items

- GDB stub debugging (`-s -S`) — wire a `cargo xtask gdb` once we have a
  debugger story on Windows (gdb isn't installed natively; candidates:
  lldb, gdb-multiarch via a future decision)
- Limine crate 0.6.x when its nightly requirement lands on stable, or if
  we move to nightly for other reasons
- A `cargo xtask test-boot` smoke test: a `selftest` feature makes the kernel
  assert, print, and exit QEMU through `isa-debug-exit`, which is an ordinary
  port write and needs no nightly. This is the only way to get interrupts,
  paging, and context switching under automated test — worth building during
  Phase 2 (`005-code-conventions.md`)
