// mlibc sysdeps for The Wizard's OS: the sixteen functions inspected by D-114.
//
// D-74 asked for this immediately after the ABI stabilised, while changing it
// was still cheap. This file is not a compiled port, and D-114 recorded no
// mlibc revision, so it is a pressure-test transcription rather than current
// upstream source.
//
// D-181 records two independent states for every function; D-182 narrows the
// trap-sensitive teardown rows:
//
//   BODY       WIRED, COMPOSED, PARTIAL, or STUB.
//   REMAINDER  WORKS, COMPAT, DOOR, PARTIAL, or MISSING.
//
// BODY describes this file. REMAINDER describes the platform work left. The
// exact register and definitions are in `userland/README.md`.

#include <mlibc/all-sysdeps.hpp>
#include <mlibc/debug.hpp>
#include <stdint.h>
#include <time.h>

#include "wizard_abi.h"

namespace mlibc {

namespace {

// The one gate ring 3 has (D-96).
inline long call1(long number, long a) {
	long result;
	asm volatile("int $0x80" : "=a"(result) : "a"(number), "D"(a) : "memory");
	return result;
}

// A negative result is an errno negated, which is what `abis/linux` expects.
inline int as_errno(long result) {
	return result < 0 ? static_cast<int>(-result) : 0;
}

inline uint64_t read_counter() {
	uint32_t low;
	uint32_t high;
	asm volatile("lfence\n\trdtsc\n\tlfence"
	             : "=a"(low), "=d"(high)
	             :
	             : "memory");
	return (static_cast<uint64_t>(high) << 32) | low;
}

// The eventual implementation must share the ring client and correlate its
// token without consuming an unrelated out-of-order completion.
inline int monotonic_fallback(time_t *secs, long *nanos) {
	(void)secs;
	(void)nanos;
	return WIZ_ENOSYS;
}

} // namespace

// --- Process and clock -------------------------------------------------

// sys_libc_log. BODY=STUB; REMAINDER=PARTIAL.
void sys_libc_log(const char *message) {
	// Console and terminal pieces exist, but no native write operation joins
	// them. Inventing an opcode here would reserve ABI by comment.
	(void)message;
}

// sys_libc_panic. BODY=COMPOSED; REMAINDER=PARTIAL.
[[noreturn]] void sys_libc_panic() {
	sys_libc_log("mlibc: panic\n");
	sys_exit(127);
	__builtin_unreachable();
}

// sys_exit. BODY=WIRED; REMAINDER=PARTIAL.
[[noreturn]] void sys_exit(int status) {
	// Only this thread retires, and the status is discarded. Process-wide
	// lifecycle and an observer do not exist yet.
	call1(WIZ_CALL_EXIT, status);
	__builtin_unreachable();
}

// sys_clock_get. BODY=PARTIAL; REMAINDER=PARTIAL.
int sys_clock_get(int clock, time_t *secs, long *nanos) {
	if(clock != CLOCK_MONOTONIC)
		return WIZ_EINVAL;

	auto *constants = reinterpret_cast<volatile const wiz_time_constants *>(
	    WIZ_TIME_ADDRESS);
	if(constants->version != WIZ_TIME_VERSION
	    || constants->page_bytes
	           < WIZ_TIME_CLOCK_AT + sizeof(wiz_time_clock)
	    || !constants->counter_hz)
		return monotonic_fallback(secs, nanos);

	auto *c = reinterpret_cast<volatile const wiz_time_clock *>(
	    WIZ_TIME_ADDRESS + WIZ_TIME_CLOCK_AT);

	// A writer stuck odd is a broken page, not permission to spin forever.
	for(int attempt = 0; attempt < 64; attempt++) {
		uint32_t before = c->seq;
		if(before & 1u)
			continue;
		__atomic_thread_fence(__ATOMIC_ACQUIRE);

		uint32_t mode = c->mode;
		uint64_t base = c->base_ns;
		uint64_t fraction = c->fraction;
		uint64_t last = c->last_ticks;
		uint64_t multiplier = c->multiplier;
		uint32_t shift = c->shift;
		uint64_t stamp =
		    mode == WIZ_TIME_MODE_COUNTER ? read_counter() : 0;

		__atomic_thread_fence(__ATOMIC_ACQUIRE);
		uint32_t after = c->seq;
		if(before != after || (after & 1u))
			continue;

		if(mode != WIZ_TIME_MODE_COUNTER || !multiplier || shift > 63
		    || fraction >= (uint64_t{1} << shift))
			return monotonic_fallback(secs, nanos);

		uint64_t ticks = stamp - last;
		unsigned __int128 advanced =
		    static_cast<unsigned __int128>(fraction)
		    + static_cast<unsigned __int128>(ticks) * multiplier;
		unsigned __int128 whole = advanced >> shift;

		uint64_t ns;
		if(whole > UINT64_MAX) {
			ns = UINT64_MAX;
		} else {
			uint64_t delta = static_cast<uint64_t>(whole);
			ns = base > UINT64_MAX - delta ? UINT64_MAX : base + delta;
		}
		*secs = static_cast<time_t>(ns / 1000000000ull);
		*nanos = static_cast<long>(ns % 1000000000ull);
		return 0;
	}

	return monotonic_fallback(secs, nanos);
}

// --- File descriptors -------------------------------------------------

// A native handle is not by itself a POSIX descriptor. The compatibility
// layer must provide chosen descriptor numbers, shared cursors and inheritance.

// sys_read. BODY=STUB; REMAINDER=COMPAT.
int sys_read(int fd, void *buffer, size_t size, ssize_t *bytes_read) {
	// Native Read takes an explicit offset. POSIX read advances shared open-file
	// state, so the ring operation alone is not this function.
	(void)fd;
	(void)buffer;
	(void)size;
	(void)bytes_read;
	return WIZ_ENOSYS;
}

// sys_write. BODY=STUB; REMAINDER=MISSING.
int sys_write(int fd, const void *buffer, size_t size, ssize_t *bytes_written) {
	// No VFS or ring write operation exists.
	(void)fd;
	(void)buffer;
	(void)size;
	(void)bytes_written;
	return WIZ_ENOSYS;
}

// sys_seek. BODY=STUB; REMAINDER=COMPAT.
int sys_seek(int fd, off_t offset, int whence, off_t *new_offset) {
	// The native file protocol is cursorless. The POSIX adapter must own and
	// share the cursor that both this function and sys_read use.
	(void)fd;
	(void)offset;
	(void)whence;
	(void)new_offset;
	return WIZ_ENOSYS;
}

// sys_close. BODY=STUB; REMAINDER=MISSING.
int sys_close(int fd) {
	(void)fd;
	return WIZ_ENOSYS;
}

// sys_open. BODY=STUB; REMAINDER=PARTIAL.
int sys_open(const char *path, int flags, mode_t mode, int *fd) {
	// Read-only OpenAt covers existing paths. Compatibility supplies root, cwd
	// and descriptors, and rejects create, truncate and write modes as EROFS.
	(void)path;
	(void)flags;
	(void)mode;
	(void)fd;
	return WIZ_ENOSYS;
}

// --- Memory -----------------------------------------------------------

// sys_vm_map. BODY=STUB; REMAINDER=PARTIAL.
int sys_vm_map(void *hint, size_t size, int prot, int flags, int fd,
               off_t offset, void **window) {
	// Loader mapping primitives exist, but a user mapping operation cannot be
	// exposed until allocation and trap-context rules are settled. W^X remains
	// non-negotiable (D-12, D-100).
	(void)hint;
	(void)size;
	(void)prot;
	(void)flags;
	(void)fd;
	(void)offset;
	(void)window;
	return WIZ_ENOSYS;
}

// sys_vm_unmap. BODY=STUB; REMAINDER=PARTIAL.
int sys_vm_unmap(void *window, size_t size) {
	// AddressSpace::unmap exists, but its flush and reclamation obligations need
	// a trap-safe user execution route (D-88, D-182).
	(void)window;
	(void)size;
	return WIZ_ENOSYS;
}

// sys_anon_allocate. BODY=STUB; REMAINDER=PARTIAL.
int sys_anon_allocate(size_t size, void **window) {
	(void)size;
	(void)window;
	return WIZ_ENOSYS;
}

// sys_anon_free. BODY=STUB; REMAINDER=PARTIAL.
int sys_anon_free(void *window, size_t size) {
	// This inherits sys_vm_unmap's unresolved execution contract.
	(void)window;
	(void)size;
	return WIZ_ENOSYS;
}

// --- Threads and futexes ---------------------------------------------

// sys_futex_wait. BODY=STUB; REMAINDER=MISSING.
int sys_futex_wait(int *pointer, int expected, const struct timespec *time) {
	// WaitQueue closes an internal lost-wakeup window; it is not an operation
	// keyed by a validated userspace atomic or its lifetime.
	(void)pointer;
	(void)expected;
	(void)time;
	return WIZ_ENOSYS;
}

// sys_futex_wake. BODY=STUB; REMAINDER=MISSING.
int sys_futex_wake(int *pointer) {
	// No userspace-address-keyed wake operation exists.
	(void)pointer;
	return WIZ_ENOSYS;
}

// sys_tcb_set. BODY=WIRED; REMAINDER=WORKS.
int sys_tcb_set(void *pointer) {
	// The kernel range-checks the base against WIZ_USER_END and saves it with
	// the other state that follows a thread (D-118).
	return as_errno(call1(WIZ_CALL_TCB_SET, reinterpret_cast<long>(pointer)));
}

} // namespace mlibc
