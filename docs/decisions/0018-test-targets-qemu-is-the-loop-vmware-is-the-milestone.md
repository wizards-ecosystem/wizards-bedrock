# D-18 Test targets: QEMU is the loop, VMware is the milestone

Status: Accepted, 2026-07-18.

**Context.** Isaac wanted VMware as the primary test target; QEMU's
debugging story argued for a role.

**Decision.** QEMU (q35, bundled EDK2 UEFI firmware, serial to stdio) is the
minute-to-minute dev loop: `cargo xtask run`. VMware Workstation (EFI
firmware, serial to `vm/serial.log`, driven by hand or via `vmrun`) is the
end-of-phase "does it feel real" gate: `cargo xtask
vmware`. Every phase's definition-of-done includes a VMware boot.

**Consequences.** Two firmware and framebuffer paths run continuously:
EDK2/QEMU and VMware EFI/SVGA-II. NIC, GPU and storage drivers do not yet
exist, so their two-target obligations begin with their own phases. VMware
has no virtio, which is why Phase 6 chooses NVMe for storage (`010` §10).
GDB-stub debugging is an open item (no native Windows gdb yet).
