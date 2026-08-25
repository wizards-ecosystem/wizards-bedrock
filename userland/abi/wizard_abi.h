/* The Wizard's OS native ABI, as a C header.
 *
 * Written for slice 10 (D-74, D-114): a POSIX libc is the first consumer of
 * this ABI that is not the kernel's own checks, and the point of writing it
 * now is to find out whether one can sit on it *while the ABI is still cheap
 * to change*.
 *
 * Two planes, which is `002` pillar 2:
 *
 *   - The **data path** is the ring. Operations are posted to a shared page,
 *     the kernel completes them out of order, and every one reports exactly
 *     once (D-79, D-113).
 *   - A small set of **synchronous calls** covers setup, handle operations,
 *     memory mapping and blocking waits. They go through the one gate ring 3
 *     has, which is an interrupt gate at DPL 3 (D-96).
 *
 * Everything here is reached through a handle carrying rights (pillar 1).
 * There are no global namespaces: a process's power is exactly its handle
 * table, and it starts with one entry — its bootstrap channel (D-111).
 *
 * **What the kernel implements today is a subset**, and the header says which
 * so that a sysdep can be written against the whole shape and fail loudly on
 * the parts that are not there yet. That is deliberate: a header that only
 * described what exists would not have surfaced the questions this slice was
 * written to ask.
 */

#ifndef WIZARD_ABI_H
#define WIZARD_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------ *
 * Handles
 * ------------------------------------------------------------------ */

/* A handle is a process-local integer. Its low two bits are reserved and
 * always set, which is what forbids a dense encoding — and a dense encoding is
 * what would make allocation non-commutative (D-78, D-70 rule 7). There is no
 * lowest-free promise: two allocations in either order must give the same two
 * handles, so nothing may depend on which one is smaller. */
typedef uint32_t wiz_handle_t;

/* The value no handle ever has. Not zero: zero is a plausible mistake and this
 * is not, so a forgotten initialisation fails rather than naming slot zero. */
#define WIZ_HANDLE_INVALID ((wiz_handle_t)0)

/* Rights, in one word (D-78). The universal four mean the same for every kind
 * of object; the rest are per-type. The rule that keeps this list short: a
 * right must correspond to a distinction the kernel actually makes about that
 * object, never to a syscall name — which is exactly what WASI's rights
 * violated before Preview 2 deleted them. */
typedef uint32_t wiz_rights_t;

#define WIZ_RIGHT_DUPLICATE ((wiz_rights_t)(1u << 0))
#define WIZ_RIGHT_TRANSFER  ((wiz_rights_t)(1u << 1))
#define WIZ_RIGHT_WAIT      ((wiz_rights_t)(1u << 2))
#define WIZ_RIGHT_INSPECT   ((wiz_rights_t)(1u << 3))
#define WIZ_RIGHT_READ      ((wiz_rights_t)(1u << 8))
#define WIZ_RIGHT_WRITE     ((wiz_rights_t)(1u << 9))
#define WIZ_RIGHT_EXECUTE   ((wiz_rights_t)(1u << 10))
#define WIZ_RIGHT_MAP       ((wiz_rights_t)(1u << 11))

/* ------------------------------------------------------------------ *
 * The ring
 * ------------------------------------------------------------------ */

/* Where the kernel maps a process's ring. Fixed, which costs nothing because
 * no two processes share a page table (D-106, D-113). */
#define WIZ_RING_ADDRESS ((uintptr_t)0x0000200000000000ull)

/* The header page. Every field is little-endian, and the first is a version:
 * a ring that cannot say what shape it is cannot be extended without guessing,
 * which is IoRing's improvement over io_uring (D-79). */
struct wiz_ring_header {
	uint32_t version;
	uint32_t submissions;      /* Entries in the submission queue. */
	uint32_t completions;      /* Power of two, >= 2 * submissions. */
	uint32_t reserved0;
	uint32_t submission_head;  /* Kernel writes; userspace reads. */
	uint32_t submission_tail;  /* Userspace writes; kernel reads. */
	uint32_t completion_head;  /* Userspace writes; kernel reads. */
	uint32_t completion_tail;  /* Kernel writes; userspace reads. */
};

#define WIZ_RING_VERSION 1u

/* Entries begin on the second page. Submissions first, then completions. */
#define WIZ_RING_ENTRIES_AT 4096u

struct wiz_submission {
	uint32_t opcode;
	uint32_t token;   /* The caller's tag, returned in the completion. */
	uint64_t argument;
};

struct wiz_completion {
	uint32_t token;   /* The submission's tag. */
	uint32_t status;  /* Zero, or a refusal. */
	uint64_t value;
};

/* Opcodes. Minimal on purpose and with no privileged ones: io_uring's exploit
 * record is a warning about surface growth, and Google's kernelCTF disables it
 * outright (`002`). */
#define WIZ_OP_ECHO   1u
#define WIZ_OP_UPTIME 2u

/* Completion statuses. */
#define WIZ_STATUS_OK             0u
#define WIZ_STATUS_UNKNOWN_OPCODE 1u

/* ------------------------------------------------------------------ *
 * Synchronous calls
 * ------------------------------------------------------------------ */

/* The one gate ring 3 has. An interrupt gate at DPL 3 rather than a trap gate,
 * because `libs/x86-tables` offers no trap gate and the interrupt gate clears
 * the interrupt flag on entry, which is what the kernel wants anyway (D-96). */
#define WIZ_SYSCALL_VECTOR 0x80

/* Call numbers, in `rax`. Arguments in `rdi`, `rsi`, `rdx`, `r10`; the result
 * in `rax`. A negative result is a `wiz_errno_t` negated, which is Linux's
 * convention and is adopted here because `abis/linux` is what mlibc brings
 * with it — the *userspace-visible* ABI becomes Linux's, and the native one is
 * unaffected (D-74). */
enum wiz_call {
	/* Implemented. */
	WIZ_CALL_EXIT = 0,
	WIZ_CALL_RING_ENTER = 1,    /* Serve what has been submitted. */
	WIZ_CALL_TCB_SET = 2,       /* Point `fs:` at thread-local storage. */

	/* Owed. Every one of these has a kernel-side mechanism already and no
	 * ring-3 entry point; the sysdeps name them so that what is missing is
	 * a refusal the caller can see and not a silent wrong answer (D-114). */
	WIZ_CALL_HANDLE_CLOSE = 3,
	WIZ_CALL_HANDLE_DUPLICATE = 4,
	WIZ_CALL_CHANNEL_SEND = 5,
	WIZ_CALL_CHANNEL_RECEIVE = 6,
	WIZ_CALL_VM_MAP = 7,
	WIZ_CALL_VM_UNMAP = 8,
	WIZ_CALL_CLOCK_GET = 9,
	WIZ_CALL_FUTEX_WAIT = 10,
	WIZ_CALL_FUTEX_WAKE = 11,
};

/* Errors, as Linux numbers them, because that is what `abis/linux` brings. */
typedef int wiz_errno_t;

/* `sys_tcb_set` takes a base below WIZ_USER_END and refuses anything else:
 * writing a non-canonical value to FS_BASE is a fault in *kernel* context,
 * which would be a program crashing the handler rather than itself (D-118). */
#define WIZ_USER_END ((uint64_t)0x0000800000000000ull)

#define WIZ_ENOSYS 38
#define WIZ_EINVAL 22
#define WIZ_EBADF  9
#define WIZ_EPERM  1
#define WIZ_EAGAIN 11
#define WIZ_ENOMEM 12

/* ------------------------------------------------------------------ *
 * Signals
 * ------------------------------------------------------------------ */

/* A signal is **a completion entry** (D-80). There is no second event plane
 * and no in-kernel signal frame: a blocking ring wait is interruptible because
 * a wakeup is indistinguishable from a completion, which is the same mechanism
 * rather than another one.
 *
 * What this costs a libc is stated here rather than discovered later:
 * `sigaction` runs entirely in userspace off a completion, so a handler is
 * dispatched by the libc's own ring loop and never by the kernel pushing a
 * frame. A program that blocks in a call which is *not* a ring wait cannot be
 * interrupted at all — which is why every blocking libc entry point below is
 * written as a ring submission plus a wait. */
#define WIZ_OP_SIGNAL 3u  /* Owed: the opcode a signal arrives as. */

#ifdef __cplusplus
}
#endif

#endif /* WIZARD_ABI_H */
