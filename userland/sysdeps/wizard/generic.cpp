// mlibc sysdeps for The Wizard's OS: the sixteen mandatory tags.
//
// D-74 asked for this "immediately after the ABI stabilises, not to run
// software, but to find out whether a POSIX libc can sit on this ABI while the
// ABI is still cheap to change". This file is the finding-out. It is **not
// compiled** — see `userland/README.md` for the toolchain wall and what it
// cost — so every line here is a claim about the ABI rather than a tested
// one, and each is marked with which of the three it is:
//
//   MAPS      — the native ABI already has the shape POSIX wants.
//   OWED      — the mechanism exists in the kernel and has no ring-3 door.
//   ABSENT    — the native ABI has nothing of the kind, deliberately.
//
// The counts are in the README, and the ones marked ABSENT are the ones worth
// arguing about, because each is a place where POSIX assumes something a
// capability system does not have.

#include <mlibc/all-sysdeps.hpp>
#include <mlibc/debug.hpp>

#include "wizard_abi.h"

namespace mlibc {

namespace {

// The one gate ring 3 has (D-96). Written out rather than wrapped, because a
// libc's syscall stub is the one place where the register discipline is the
// whole interface.
inline long call0(long number) {
	long result;
	asm volatile("int $0x80" : "=a"(result) : "a"(number) : "memory");
	return result;
}

inline long call1(long number, long a) {
	long result;
	asm volatile("int $0x80" : "=a"(result) : "a"(number), "D"(a) : "memory");
	return result;
}

inline long call3(long number, long a, long b, long c) {
	long result;
	asm volatile("int $0x80"
	             : "=a"(result)
	             : "a"(number), "D"(a), "S"(b), "d"(c)
	             : "memory");
	return result;
}

// A negative result is an errno negated, which is what `abis/linux` expects.
inline int as_errno(long result) {
	return result < 0 ? static_cast<int>(-result) : 0;
}

// The ring, at the address the kernel maps it (D-113). A libc gets it for
// free: it is in the address space before the first instruction runs, so
// there is no "open the ring" call to fail.
inline volatile wiz_ring_header *ring() {
	return reinterpret_cast<volatile wiz_ring_header *>(WIZ_RING_ADDRESS);
}

} // namespace

// --- The three that need nothing but the gate, and one that has no door ---

// sys_libc_log. MAPS **in shape only**, which is how the README counts it and
// how this comment did not. The kernel's console is not a file and a libc does
// not need it to be: this is diagnostics, and a ring opcode taking a pointer
// and a length is the whole of what it wants.
void sys_libc_log(const char *message) {
	// The body is a no-op and says so. It used to claim it submitted
	// something the kernel refused with STATUS_UNKNOWN_OPCODE — it does not
	// submit anything, so a libc built today gets silence rather than the
	// visible failure that comment promised (D-124).
	//
	// OWED: WIZ_OP_LOG. Deliberately not faked with a refused submission:
	// an opcode number invented here would be one the kernel has never
	// heard of, and the first real one would have to avoid it.
	(void)message;
}

// sys_libc_panic. MAPS. A libc that has lost its invariants exits; there is no
// "abort into the kernel" and there does not need to be, because a ring-3
// fault already retires the thread and nothing else (D-102).
[[noreturn]] void sys_libc_panic() {
	sys_libc_log("mlibc: panic\n");
	sys_exit(127);
	__builtin_unreachable();
}

// sys_exit. MAPS, and is the only one of the sixteen that is implemented on
// the kernel side today. A ring-3 thread that calls in is retired from the far
// side of the boundary (D-96); the status is accepted and currently discarded,
// which is a gap a parent waiting on a child would notice and nothing else
// does yet.
[[noreturn]] void sys_exit(int status) {
	call1(WIZ_CALL_EXIT, status);
	__builtin_unreachable();
}

// sys_clock_get. OWED, and under the heading above rather than beside the
// other OWED entries because it is the last of the four a libc reaches
// through the bare gate. `libs/timebase` already converts the TSC to nanoseconds
// and the ring has an uptime opcode, so the mechanism is complete and the door
// is missing. Note what POSIX wants that the kernel does not have: a *wall
// clock*. Uptime is monotonic; `CLOCK_REALTIME` needs an epoch, and this
// kernel has never been told what time it is.
int sys_clock_get(int clock, time_t *secs, long *nanos) {
	long value = 0;
	long result = call3(WIZ_CALL_CLOCK_GET, clock,
	                    reinterpret_cast<long>(&value), 0);
	if(result < 0)
		return as_errno(result);
	*secs = value / 1000000000L;
	*nanos = value % 1000000000L;
	return 0;
}

// --- The file-descriptor five: four OWED, and `open` ---------------------

// A POSIX fd is a handle (pillar 1), and the translation is exact for the
// operations below: a descriptor number *is* a handle value, and the rights
// on it are what `open` asked for. What POSIX adds that handles do not have is
// the *table's* semantics — dup2 to a chosen number, and inheritance across
// exec. Neither is among the sixteen mandatory tags; both are in the README's
// "cluster that stays unimplementable", which is where this comment used to
// say "further down" and point at nothing.

// sys_read. OWED, over the ring rather than as a synchronous call.
int sys_read(int fd, void *buffer, size_t size, ssize_t *bytes_read) {
	// OWED: WIZ_CALL_RING_ENTER with a read opcode. Written as a ring
	// submission and not as a synchronous call on purpose: `002` pillar 2
	// makes the data path asynchronous, and a blocking read is a submission
	// plus a wait. That is also what makes it interruptible by a signal,
	// because a signal *is* a completion (D-80).
	(void)fd;
	(void)buffer;
	(void)size;
	(void)bytes_read;
	return WIZ_ENOSYS;
}

// sys_write. OWED, as sys_read.
int sys_write(int fd, const void *buffer, size_t size, ssize_t *bytes_written) {
	// OWED, as sys_read.
	(void)fd;
	(void)buffer;
	(void)size;
	(void)bytes_written;
	return WIZ_ENOSYS;
}

// sys_seek. OWED, and the state it moves belongs to the object.
int sys_seek(int fd, off_t offset, int whence, off_t *new_offset) {
	// OWED. A seek is state the *file object* holds, not the handle: two
	// handles to one file share a position in POSIX, which is a distinction
	// the kernel will have to make about the object and therefore a
	// legitimate per-type right (D-78's rule).
	(void)fd;
	(void)offset;
	(void)whence;
	(void)new_offset;
	return WIZ_ENOSYS;
}

// sys_close. OWED: the kernel side exists and has no ring-3 door.
int sys_close(int fd) {
	// OWED: WIZ_CALL_HANDLE_CLOSE. The kernel side exists — closing advances
	// the slot's generation so the value is dead the instant it is closed
	// (D-78) — and there is no ring-3 door to it.
	return as_errno(call1(WIZ_CALL_HANDLE_CLOSE, fd));
}

// sys_open. ABSENT: it needs a namespace this OS deliberately has not got.
int sys_open(const char *path, int flags, mode_t mode, int *fd) {
	// ABSENT — the only one of the sixteen, and the README has always said
	// so while this line said OWED. The distinction is the whole point of
	// the taxonomy: OWED means the kernel has the mechanism and no door,
	// and there is no mechanism here to have a door onto. What it needs is
	// a *namespace*. There is no global filesystem to name a path in, by
	// design (pillar 1), so `open` must resolve through a directory handle
	// the process was given — which is `openat` with no `AT_FDCWD`, and is
	// what WASI's `path_open` settled on for the same reason.
	//
	// A libc can be built this way. What it cannot do is `open("/etc/foo")`
	// from a process nobody gave a root directory to, and that is the point.
	(void)path;
	(void)flags;
	(void)mode;
	(void)fd;
	return WIZ_ENOSYS;
}

// --- OWED: memory ------------------------------------------------------

// sys_vm_map. OWED, over D-107's map_user, with W^X non-negotiable.
int sys_vm_map(void *hint, size_t size, int prot, int flags, int fd,
               off_t offset, void **window) {
	// OWED: WIZ_CALL_VM_MAP. The kernel side is D-107's `map_user`, and the
	// type discipline maps onto POSIX cleanly — a `MAP_ANONYMOUS` mapping is
	// a fresh `USegment`, and a file mapping is one the file object owns.
	//
	// W^X is **not** negotiable (D-12, D-100): `PROT_WRITE | PROT_EXEC` is
	// refused rather than granted, and a JIT will have to map twice. That is
	// a real incompatibility with some ported software and it is the right
	// one to have.
	(void)hint;
	(void)size;
	(void)prot;
	(void)flags;
	(void)fd;
	(void)offset;
	(void)window;
	return WIZ_ENOSYS;
}

// sys_vm_unmap. OWED, over unmap and its flush token (D-88).
int sys_vm_unmap(void *window, size_t size) {
	// OWED: WIZ_CALL_VM_UNMAP. `AddressSpace::unmap` exists and returns a
	// flush token (D-88), which is the shape a shootdown will need.
	(void)window;
	(void)size;
	return WIZ_ENOSYS;
}

// sys_anon_allocate. OWED, and is sys_vm_map with the anonymous flag.
int sys_anon_allocate(size_t size, void **window) {
	return sys_vm_map(nullptr, size, 3 /* read|write */, 0x20 /* anonymous */,
	                  -1, 0, window);
}

// sys_anon_free. OWED, and is sys_vm_unmap.
int sys_anon_free(void *window, size_t size) {
	return sys_vm_unmap(window, size);
}

// --- OWED: threads and futexes -----------------------------------------

// sys_futex_wait. OWED, over the wait queue that already closes the window.
int sys_futex_wait(int *pointer, int expected, const struct timespec *time) {
	// OWED: WIZ_CALL_FUTEX_WAIT. `kernel/src/wait.rs` already has the hard
	// half — `wait_unless` registers before it re-reads, which is what makes
	// a futex free of lost wakeups (D-95), and is the same window a futex
	// gets wrong first.
	(void)pointer;
	(void)expected;
	(void)time;
	return WIZ_ENOSYS;
}

// sys_futex_wake. OWED, over WaitQueue::wake_all.
int sys_futex_wake(int *pointer) {
	// OWED: WIZ_CALL_FUTEX_WAKE, over `WaitQueue::wake_all`.
	(void)pointer;
	return WIZ_ENOSYS;
}

// sys_tcb_set. MAPS, since D-118 — and this sysdep is why.
int sys_tcb_set(void *pointer) {
	// Writing it found that
	// `FS_BASE` is a *fourth* thing that must follow a thread, after the
	// privilege stack (D-99), the tables (D-106) and the extended state
	// (D-108). Nothing native has thread-local storage, so nothing native
	// would have asked.
	//
	// The kernel range-checks the base against WIZ_USER_END rather than
	// against canonicality alone: a program asking to point `fs:` at kernel
	// memory has no reason it could be given.
	return as_errno(call1(WIZ_CALL_TCB_SET, reinterpret_cast<long>(pointer)));
}

} // namespace mlibc
