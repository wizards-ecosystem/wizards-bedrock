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

// --- MAPS: the four that need nothing but the gate ---------------------

// sys_libc_log. MAPS. The kernel's console is not a file and a libc does not
// need it to be: this is diagnostics, and the ring's echo opcode is enough to
// prove the path. Owed a real opcode that takes a pointer and a length.
void sys_libc_log(const char *message) {
	// OWED: WIZ_OP_LOG. Until it exists this is a submission the kernel
	// refuses with STATUS_UNKNOWN_OPCODE, which is *reported* rather than
	// ignored (D-79) — so a libc built today gets a failure it can see.
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

// sys_clock_get. OWED. `libs/timebase` already converts the TSC to nanoseconds
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

// --- OWED: the file-descriptor four ------------------------------------

// A POSIX fd is a handle (pillar 1), and the translation is exact for the
// operations below: a descriptor number *is* a handle value, and the rights
// on it are what `open` asked for. What POSIX adds that handles do not have is
// the *table's* semantics — dup2 to a chosen number, and inheritance across
// exec — and both are recorded as ABSENT further down.

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

int sys_write(int fd, const void *buffer, size_t size, ssize_t *bytes_written) {
	// OWED, as sys_read.
	(void)fd;
	(void)buffer;
	(void)size;
	(void)bytes_written;
	return WIZ_ENOSYS;
}

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

int sys_close(int fd) {
	// OWED: WIZ_CALL_HANDLE_CLOSE. The kernel side exists — closing advances
	// the slot's generation so the value is dead the instant it is closed
	// (D-78) — and there is no ring-3 door to it.
	return as_errno(call1(WIZ_CALL_HANDLE_CLOSE, fd));
}

int sys_open(const char *path, int flags, mode_t mode, int *fd) {
	// OWED, and the one that needs the most that does not exist: a
	// *namespace*. There is no global filesystem to name a path in, by
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

int sys_vm_unmap(void *window, size_t size) {
	// OWED: WIZ_CALL_VM_UNMAP. `AddressSpace::unmap` exists and returns a
	// flush token (D-88), which is the shape a shootdown will need.
	(void)window;
	(void)size;
	return WIZ_ENOSYS;
}

int sys_anon_allocate(size_t size, void **window) {
	return sys_vm_map(nullptr, size, 3 /* read|write */, 0x20 /* anonymous */,
	                  -1, 0, window);
}

int sys_anon_free(void *window, size_t size) {
	return sys_vm_unmap(window, size);
}

// --- OWED: threads and futexes -----------------------------------------

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

int sys_futex_wake(int *pointer) {
	// OWED: WIZ_CALL_FUTEX_WAKE, over `WaitQueue::wake_all`.
	(void)pointer;
	return WIZ_ENOSYS;
}

int sys_tcb_set(void *pointer) {
	// OWED: WIZ_CALL_TCB_SET, which writes `FS_BASE`. The kernel has the MSR
	// wrapper already; what it does not have is saving `FS_BASE` across a
	// context switch, which is a third thing that must follow a thread
	// (D-99, D-106, D-108 are the other three) and belongs in `prepare` with
	// them.
	(void)pointer;
	return WIZ_ENOSYS;
}

} // namespace mlibc
