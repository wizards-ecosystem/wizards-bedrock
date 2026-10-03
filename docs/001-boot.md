# 001 - Boot

## Decision

Boot via **UEFI + Limine** (boot protocol base revision 4 since D-42, `limine` crate
0.5.0, pinned because 0.6.x requires nightly). Limine remains the maintained,
vendored boot boundary; an own loader has no standing roadmap slot (D-219).
It gives us:

- entry directly in 64-bit long mode, higher half
- framebuffer, memory map, HHDM offset, and the module carrying the initrd
- identical behavior on QEMU/EDK2, VMware EFI, and real UEFI hardware

The allocation-free memory-map copy is all or nothing: `libs/memory-map`
retains every reported region through the fixed 128-entry bound and returns a
typed refusal above it. Allocator, direct-map and firmware exclusion policy
therefore never act on a plausible-looking prefix (D-224).

## Kernel image contract

- ELF64, linked at `0xffffffff80000000` (linker script `kernel/linker.ld`)
- built for `x86_64-unknown-none` with `code-model=kernel`,
  `relocation-model=static` (`.cargo/config.toml`)
- Limine requests live in their own `.requests` output section, between
  `.requests_start_marker` / `.requests_end_marker`, each `KEEP()`-ed against
  section garbage collection
- three `PT_LOAD` segments (read-execute, read-only, read-write) and Limine
  maps each by its flags, so W^X holds for the kernel image from the first
  instruction, well before the kernel owns page tables of its own
- entry: `extern "C" fn kmain() -> !`

Every one of those properties is asserted after each build by
`xtask/src/elf.rs`, which reads the linked ELF back rather than trusting the
script (D-40). An unnamed section is placed by the linker's orphan heuristic
and still links. That is how `.got` spent Phase 2 sitting after `.bss`,
forcing 29 KB of zeros into every image.

The requests and the `kmain` symbol are emitted by `wost::entry!(kernel_main)`,
which expands *in the binary crate*: `#[used]` statics inside an rlib can be
dropped by the linker (D-21). The kernel itself sees only `wost::boot::BootInfo`
and never names the bootloader protocol; `wost/src/boot/limine_glue.rs` is the
whole of that dependency.

## Disk layout (built by `cargo xtask image`, pure Rust)

64 MiB raw image, MBR partition table (not GPT, deliberately minimal and
hand-written in `xtask`; GPT comes when the OS grows a partitioning story):

- partition 1: type `0xEF` (EFI System), FAT32, from LBA 2048
  - `/EFI/BOOT/BOOTX64.EFI`: Limine v10 (vendored in `boot/limine/`)
  - `/boot/limine/limine.conf`: from `boot/limine.conf`
  - `/boot/wizard-kernel.elf`: the kernel

Limine finds its config at `boot():/boot/limine/limine.conf` (first entry in
its search order) and loads the kernel per `path: boot():/boot/wizard-kernel.elf`.

## Runners

- **QEMU** (`cargo xtask run`): q35 machine, EDK2 firmware from the QEMU
  install (copied into `target/` as pflash code+vars), serial to stdio.
- **VMware** (`cargo xtask vmware`): generates the self-test machine in
  `vm/self-test/`
  with a flat-VMDK descriptor over a copy of the raw image and a
  `firmware = "efi"` VMX. The separate
  `cargo xtask vmware --shipped` machine lives in `vm/shipped/`. Serial goes
  to the generation-specific path each command prints. Each command
  prints the exact image digest and quoted `vmrun` invocation; D-244 binds its
  later execution observation.

## Verified

2026-07-18: both QEMU (1280x800 fb) and VMware (1024x768 fb) boot to the
framebuffer banner.

2026-08-12: re-verified on both after the framekernel split (D-21). QEMU at
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

2026-08-13: QEMU, after the descriptor tables landed (`006-cpu-fundamentals.md`,
D-29..D-32). The runner moved to `-cpu max` so the protection bits exist to be
enabled:

```
[info ] The Wizard's OS awakens.
[info ] framebuffer: 1280x800 pitch=5120 bpp=32
[info ] limine: base revision supported
[info ] cpu: descriptor tables installed
[info ] cpu: protections smep smap umip nx
[info ] kernel: entering idle loop
```

The framebuffer console still draws after SMAP is enabled, which is the
observation that confirms Limine's mappings are supervisor-only.

Same day on VMware Workstation (`vmrun start vm\wizard-os.vmx`, serial to
`vm/serial.log`), which is the target that matters for this slice: its virtual
CPU exposes all four protection features, and its SVGA-II framebuffer is the one
that could have differed under a supervisor-only mapping assumption.

```
[info ] The Wizard's OS awakens.
[info ] framebuffer: 1024x768 pitch=4096 bpp=32
[info ] limine: base revision supported
[info ] cpu: descriptor tables installed
[info ] cpu: protections smep smap umip nx
[info ] kernel: entering idle loop
```

The linked image was checked at the same time: entry point equals the `kmain`
symbol, the three `PT_LOAD` segments are `R-X`, `R--` and `RW-` with none both
writable and executable, and the generated stub table spans exactly 4096 bytes
at a 16-byte boundary: the assumption `interrupt::init` asserts at runtime,
confirmed statically as well.

2026-08-20: Phase 2 complete, verified on both targets. QEMU's `test-boot`
runs 34 machine checks, among them a QMP-injected keystroke decoded end to
end (D-38). VMware Workstation (`vmrun start vm\wizard-os.vmx`, serial to
`vm/serial.log`) boots the same kernel and is the reading that matters for
the timer: its APIC clock measured at 65.9 MHz by the same code that
measured QEMU's at 997 MHz: rates fifteen times apart, which is the spread
the calibration's plausibility bounds were designed around, and both landed
inside them on the first boot.

```
[info ] The Wizard's OS awakens.
[info ] framebuffer: 1024x768 pitch=4096 bpp=32
[info ] limine: base revision supported
[info ] cpu: descriptor tables installed
[info ] cpu: protections smep smap umip nx
[info ] pic: legacy controllers remapped and masked
[info ] apic: x2apic id 0 version 0x15 (bootstrap)
[info ] time: 100 hz tick: apic timer at 65929212 hz, initial count 659292
[info ] cpu: accepting interrupts
[info ] keyboard: polled, scancode set 2
[info ] kernel: entering idle loop, uptime 4 ms
```

The `uptime 4 ms` line is the interpolation visibly alive (a whole-tick
clock could only print multiples of ten), and `keyboard: polled, scancode
set 2` is VMware's i8042 surviving the full bring-up: reset, self-test,
translation off, set 2 selected. The eyeball that closes the phase is
typing at the VMware console and watching the glyphs echo under the banner.

Re-verified the same day, after the phase's adversarial audit hardened the
calibration (D-36's amendment): the paired windows agreed on both targets:
VMware at 65.93 MHz, within a part in ten thousand of the first boot's
reading, and QEMU at 999.96 MHz, dead on the rate QEMU models.

## Debug channels

- COM1 serial (`kernel/src/arch/x86_64/uart.rs`): primary log, works
  everywhere. Polled 16550 at 115200 8N1, with a loopback probe at bring-up so
  a missing controller is reported rather than silently swallowing every line.
  Records carry a level tag; `debug!` is filtered out by default.
- framebuffer console (`kernel/src/console.rs`): Noto Sans Mono bitmap
  glyphs, scrolling, pixel packing driven by the bootloader's channel masks
  rather than an assumed `0xRRGGBB` (`libs/pixel`). Carries user-facing output
  and the panic screen; diagnostics stay on serial.

Both are brought up before the Limine base revision is checked, so a machine
with neither a serial cable nor a supported bootloader still shows why it
stopped. A framebuffer that is not direct RGB, not 32bpp, or not 4-byte aligned
in address and pitch is refused with a typed reason rather than drawn on: the
alternative failure mode is every access being rejected. Once that geometry
has constructed the exact MMIO lease, an access refusal is an internal
contradiction and fails stopped before the shadow can claim a device write
which did not happen (D-249).

## Known limitations

This began as the Phase-1 close-out snapshot. Struck entries were retired by
later phases; unstruck entries describe current facts, not automatically live
work. `016-pre-wizzy-foundation.md` is the sole current backlog.

- ~~**`wost::sync::SpinLock` does not touch the interrupt flag.**~~ Fixed in
  Phase 2 slice 2, in `SpinLock` rather than at the call sites, as this entry
  asked: acquiring one clears `IF` and the guard restores it (D-34). What
  remains is the narrower hazard the flag cannot cover (a fault or an NMI
  arriving while a lock is held), which is why the trap handler still takes no
  lock (D-31).
- ~~**`Console::scroll` reads the framebuffer back.**~~: retired 2026-08-22
  (D-48). The framebuffer keeps a shadow in RAM, written alongside every device
  write and read instead of the device, so a scroll no longer touches uncached
  memory at all. That also made the mapping safe to mark write-combining
  (D-50), where reads would have been worse still.
- **Runtime logging control is deliberately absent.** `MAX_LEVEL` in
  `kernel/src/log.rs` remains a compile-time constant. PW-705 stops runtime or
  per-module controls until a concrete configuration source exists.
- **The kernel image is not relocatable.** `linker.ld` hardcodes
  `0xffffffff80000000`, which is what blocks turning on `kaslr: yes` in
  `boot/limine.conf`. KASLR remains later Phase-12 work, not a pre-Wizzy
  foundation item (`002-architecture.md`, security baseline).

## Resolved and trigger-only items

- ~~GDB stub debugging (`-s -S`)~~: resolved by D-231: the pinned native
  WinLibs GDB owns a private paused QEMU run, discovers its dynamic loopback
  endpoint and has both interactive and bounded-proof `xtask` modes.
- **Limine 0.6.x is a stopped PW-705 trigger, not open work.** Re-evaluate the
  pinned crate only when stable support or a concrete handoff need forces it;
  a newer dependency version alone is not a trigger.
- ~~Grow `kernel/src/selftest.rs` as Phase 2 lands~~: done, the phase closed
  at 34 checks, every new one broken once to prove it can go red, and the
  harness itself grew a hand: it can press a key (D-38). Those are historical
  Phase-2 totals, not the current suite size. The 2026-08-22 audit added a
  35th, for the port claim width D-39 fixed.
