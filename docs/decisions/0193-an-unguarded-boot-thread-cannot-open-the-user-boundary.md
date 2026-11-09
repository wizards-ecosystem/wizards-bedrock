# D-193 An unguarded boot thread cannot open the user boundary

Status: Accepted, 2026-08-27. Amends D-92; closes PW-107.

**Context.** D-92 deliberately kept booting on the bootloader's unguarded
stack when the guarded replacement could not be allocated. At that point no
shipped path ran untrusted code, so the decision called this a lost guard
rather than a lost boot. D-116 later made a user program part of every shipped
boot without revisiting the fallback. `boot_continue` did not remember which
stack path reached it, and `user::spawn` therefore built a process and opened
ring 3 even when the trusted kernel itself lacked the stack boundary the
security baseline promises.

**Decision.** The boot continuation records whether it was reached through
`continue_on_stack` with a `MappedStack`. The state begins false, and any
unrecognised continuation value also records false. Process construction reads
that boot-global state before allocating an address space and returns the typed
`UnguardedBootStack` refusal unless it is true.

Failure to map the stack remains a degraded boot rather than a panic. The
existing mapping error and the fallback warning diagnose it, the console,
interrupt and storage paths may continue, and the shipped launch reports that
ring 3 is closed. Only the crate root can mark the production state true; the
self-test build can temporarily inject the false state around one attempted
launch and then restores it.

**Rationale.** The guard protects the kernel while it handles untrusted work,
so losing it must revoke that work rather than every diagnostic service. A
panic would be fail-closed but would throw away the machine's ability to say
what failed. Keeping the earlier fallback unchanged would make a best-effort
allocation silently weaken a privilege boundary. Retrying allocation cannot
guarantee progress under the memory pressure which caused the first refusal.
A boot-global fact is smaller than threading a token through every later
process caller, and its sole production writer is the continuation that knows
which path actually ran.

**Consequences.** `an unguarded boot stack closes the user boundary` injects
the exact state left by `boot_continue(0)`, requires the typed refusal, no new
syscall crossing and an unchanged frame, heap, arena, channel, ring, thread and
process-record ledger. Restoring the state lets all later user checks run,
which proves the degraded state is controllable rather than sticky test damage.
Bypassing the process gate launched a real ring-3 program and reddened only
that check with `passed=197 failed=1 expected=198`; it is mutation 101. No
unsafe operation was added. The image now carries 198 machine checks.
