# D-231 A debugger owns one paused machine and discovers its endpoint

Status: Accepted, 2026-08-29. Closes PW-513.

**Decision.** The supported route is the exact native GDB identity shipped by
WinLibs POSIX/UCRT `16.1.0-14.0.0-r4`: `GNU gdb (GDB for MinGW-W64 x86_64,
built by Brecht Sanders, r4) 17.2`. `GDB` may name its executable explicitly;
otherwise the task searches `PATH` and the WinGet package directory. A changed
identity is refused until it is reviewed rather than silently becoming the
debugger baseline.

`cargo xtask debug` builds the unoptimised shipped kernel, starts one headless
QEMU machine paused at reset and opens GDB with that ELF's symbols. The run
owns a private boot image and firmware-variable store, snapshots every shared
disk and always terminates and reaps QEMU when GDB exits. QEMU dials an HMP
listener the harness already owns. HMP starts the GDB server on loopback port
zero and `info chardev` reports the assigned address; no fixed port or
probe/rebind interval exists. The initial command uses a hardware breakpoint
at the linker entry `kmain`, continues to it and leaves the interactive prompt
there.

`cargo xtask debug --check` is the non-interactive acceptance path. It has a
fixed deadline, requires GDB to hit `kmain`, emit an exact marker and produce a
symbolized frame naming `wizard_kernel::kmain` at `wost/src/boot/mod.rs`, then
detaches. Success, refusal and timeout all converge on the same QEMU cleanup.
The debugger remains an exceptional developer dependency and does not enter
CI; the host gate checks its command construction, endpoint parsing, pinned
identity and proof-output parser without requiring GDB.

**Rationale.** The documented Windows workflow stopped at a raw `-s -S`
suggestion, leaving debugger selection, symbol loading, the endpoint and
cleanup to every investigation. A fixed TCP port conflicts with concurrent
runners, and probing a free port before QEMU binds it repeats the race D-227
removed from the other machine controls. Rust's `rust-lldb` launcher is
installed here but no LLDB executable is; the native WinLibs package does
provide GDB 17.2.

**Alternatives.** LLDB was rejected for the present setup because a Cargo
wrapper without the debugger it launches is not a tool. WSL or
`gdb-multiarch` was rejected because D-16's build is native Windows and a
second environment buys nothing now that native GDB reads the kernel ELF.
QEMU's conventional port 1234 and a guessed free port were rejected because
both make otherwise independent runners contend. Leaving QEMU alive after a
debugger failure was rejected because an overnight or automated proof must not
strand a paused machine.

**Consequences.** Mutations 272 through 276 keep the host checks red for
accepting port zero, an unreviewed GDB identity, a software breakpoint before
the higher-half entry was mapped, a backtrace missing its symbol or source, and
a renamed documented task. No target code, unsafe operation, wire ABI or
machine check changed.
