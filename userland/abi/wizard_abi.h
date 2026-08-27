/* The Wizard's OS native ABI, transcribed for a C consumer.
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
 *   - Four **synchronous calls** cover exit, the ring doorbell and wait, and
 *     thread-local-storage setup. They go through the one gate ring 3 has,
 *     which is an interrupt gate at DPL 3 (D-96).
 *
 * Object authority is reached through a handle carrying rights (pillar 1).
 * Handleless ring/control operations, the ring mapping and the time page are
 * enumerated ambient facilities rather than objects. There is no global
 * filesystem namespace.
 *
 * At its first instruction a program's stack points at a three-word,
 * versioned entry block. Version 1 carries its launch argument and the numeric
 * identity of its sole bootstrap handle (D-182).
 *
 * Only wire values the kernel currently decodes are numbered here. D-181
 * removed proposed call and signal numbers: pressure-testing a future shape
 * is useful, but a comment cannot reserve ABI.
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

/* The value no handle ever has, and it is zero on purpose — the comment here
 * used to say "not zero" directly above a zero.
 *
 * Zero is safe to use precisely because of the two reserved low bits above:
 * every real handle carries 0b11 there, the mixer cannot clear them, and
 * `decode` refuses anything that does not. So zero is unreachable by
 * construction, and a forgotten initialisation names nothing rather than
 * naming slot zero. */
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
/* Admitted for Channel, not File: no current file write operation exists. */
#define WIZ_RIGHT_WRITE     ((wiz_rights_t)(1u << 9))
/* Raw rights defined by handle-space. No current object type admits either;
 * executable authority and user mapping remain undecided platform work. */
#define WIZ_RIGHT_EXECUTE   ((wiz_rights_t)(1u << 10))
#define WIZ_RIGHT_MAP       ((wiz_rights_t)(1u << 11))
/* May resolve a name in this directory that you already know. Distinct from
 * enumeration on purpose: "may resolve a name I know" and "may list the names"
 * are different powers, and the resolver consults one while readdir consults
 * the other. */
#define WIZ_RIGHT_TRAVERSE ((wiz_rights_t)(1u << 12))
#define WIZ_RIGHT_ENUMERATE ((wiz_rights_t)(1u << 13))
/* Raw reserved right. No current Directory admits it because no namespace-
 * mutating operation exists. */
#define WIZ_RIGHT_MODIFY ((wiz_rights_t)(1u << 14))

/* ------------------------------------------------------------------ *
 * Program entry
 * ------------------------------------------------------------------ */

/* `rsp` points here at the first instruction, and each word is little-endian.
 * Version comes first so a consumer can refuse an entry shape it does not
 * understand before assigning meaning to either following word. A numeric
 * handle is not a second grant: authority still comes from the one live entry
 * in this process's table. */
struct wiz_entry_block {
	uint64_t version;
	uint64_t argument;
	uint64_t bootstrap_handle; /* Must fit `wiz_handle_t`. */
};

#define WIZ_ENTRY_VERSION             1u
#define WIZ_ENTRY_VERSION_AT          0u
#define WIZ_ENTRY_ARGUMENT_AT         8u
#define WIZ_ENTRY_BOOTSTRAP_HANDLE_AT 16u
#define WIZ_ENTRY_BYTES               24u

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

/* The data area: a third page, and the buffer every operation that moves bytes
 * uses. **The ring owns it**, so it is always mapped, always the right size,
 * and its bounds are arithmetic — there is no pointer of yours for the kernel
 * to validate, no SMAP window to open and no fault to recover.
 *
 * The cost is a copy: a read goes device -> kernel -> here, where your own
 * buffer could have been the target. Zero-copy needs memory objects, which is
 * a later phase (D-173). */
#define WIZ_RING_DATA_AT 8192u
#define WIZ_RING_DATA_BYTES 4096u

/* Thirty-two bytes since slice 9 widened it: a read needs a handle, an offset,
 * a buffer and a length, which is more than one argument holds. The ABI slices
 * came first so that the caller which freezes the shape would arrive after it
 * could still be changed, and `read` is that caller (D-149, D-173).
 *
 * Fields mean what the opcode says they mean, and nothing carries two things:
 *
 *   WIZ_OP_ECHO       argument is returned
 *   WIZ_OP_UPTIME     nothing
 *   WIZ_OP_DUPLICATE  handle, and argument is the rights you will hold
 *   WIZ_OP_TIMER      argument is nanoseconds
 *   WIZ_OP_CANCEL     argument is the token to end
 *   WIZ_OP_OPENAT     handle is a directory; argument/length is a path in the
 *                     data area; the completion carries the new handle
 *   WIZ_OP_READ       handle is a file; offset says where; argument/length is
 *                     where in the data area to put the bytes
 */
struct wiz_submission {
	uint32_t opcode;
	uint32_t token;   /* The caller's tag, returned in the completion. */
	uint32_t handle;  /* What the operation is about, where it is about one. */
	uint32_t length;  /* How many bytes, where that is a question. */
	uint64_t offset;  /* Where in a file. Per operation: there is no cursor. */
	uint64_t argument;
};

#define WIZ_SUBMISSION_BYTES 32u
#define WIZ_COMPLETION_BYTES 16u

struct wiz_completion {
	uint32_t token;   /* The submission's tag. */
	uint32_t status;  /* Zero, or a refusal. */
	uint64_t value;
};

/* Opcodes. Minimal on purpose and with no privileged ones: io_uring's exploit
 * record is a warning about surface growth, and Google's kernelCTF disables it
 * outright (`002`). */
#define WIZ_OP_ECHO      1u
#define WIZ_OP_UPTIME    2u
/* Duplicate a handle, narrowing its rights. `handle` names the source and
 * `argument` is the rights word requested. The completion's value is the new
 * handle. Asking for more than the source holds is refused rather than quietly
 * reduced (D-151, D-163). */
#define WIZ_OP_DUPLICATE 3u
/* Complete once `argument` nanoseconds have passed. Admitted now, completed
 * later; `WIZ_CALL_RING_WAIT` is how you wait for it without spinning. */
#define WIZ_OP_TIMER     4u
/* End an operation still in flight, naming it by its token. Its own operation
 * with its own completion: the target still gets exactly one, carrying
 * `WIZ_STATUS_CANCELLED` (D-164). */
#define WIZ_OP_CANCEL    5u
/* Resolve a path from a directory handle. `handle` is the directory,
 * `argument` is an offset into the data area and `length` the path's bytes.
 * The completion carries a handle for what the path named.
 *
 * The rights on it are **derived from the directory's**, never asked for. A
 * directory preserves its parent's same-type rights. A file preserves only
 * universal rights already held and derives READ from TRAVERSE; that cross-type
 * derivation cannot recreate a universal right. */
#define WIZ_OP_OPENAT    6u
/* Read a file into the data area. `offset` is where in the file, and it is
 * **per operation** — there is no seek, no tell and no cursor anywhere. */
#define WIZ_OP_READ      7u

/* Completion statuses. */
#define WIZ_STATUS_OK             0u
#define WIZ_STATUS_UNKNOWN_OPCODE 1u
/* Your handle table has no room for what this would produce. Reported at
 * submission, which is the point: you can close something and ask again. */
#define WIZ_STATUS_NO_HANDLE_SLOT 2u
#define WIZ_STATUS_BAD_HANDLE     3u
#define WIZ_STATUS_DENIED         4u
#define WIZ_STATUS_CANCELLED      5u
/* A cancellation named no operation in flight. Advisory, not an error: an
 * operation that finished between your submission and its service is a race
 * you cannot avoid, and io_uring reports the same case as `-EALREADY`. */
#define WIZ_STATUS_NOT_FOUND      6u
#define WIZ_STATUS_NO_ROOM        7u
/* The buffer you named is not inside the data area, or has no length. Checked
 * at submission, before anything is read: a buffer found unusable afterwards
 * is `-ENOBUFS` with the work already done. */
#define WIZ_STATUS_BAD_BUFFER     8u
#define WIZ_STATUS_NO_ENTRY       9u
/* The path is absolute, or would leave the directory. **Malformed, not
 * denied**: there is no root and no parent, so there is nothing that could
 * have been denied. */
#define WIZ_STATUS_BAD_PATH       10u

/* ------------------------------------------------------------------ *
 * The time page
 * ------------------------------------------------------------------ */

/* Where the kernel maps the clock, read-only, in every process. Reading the
 * time is a *load*, not a call (D-152, D-166). */
#define WIZ_TIME_ADDRESS ((uintptr_t)0x0000100000000000ull)

#define WIZ_TIME_VERSION 1u

/* The constants. Written before the page is mapped anywhere and never again,
 * so there is no sequence counter here and none is needed. */
struct wiz_time_constants {
	uint32_t version;
	uint32_t page_bytes;
	uint64_t counter_hz;   /* The measured rate of the counter below. */
};

/* Where the clock region begins: its own cache line, because the constants are
 * read once and the clock is read constantly. */
#define WIZ_TIME_CLOCK_AT 64u

/* The clock. Linux's seqlock: `seq` is odd while the kernel is writing, so a
 * reader takes it, checks it is even, reads the rest, and takes it again — and
 * keeps what it read only if the two agree. Both reads need a compiler
 * barrier around the fields between them. */
struct wiz_time_clock {
	uint32_t seq;
	uint32_t mode;         /* WIZ_TIME_MODE_*, and the gate on all of it. */
	uint64_t base_ns;      /* Whole nanoseconds at `last_ticks`. */
	uint64_t fraction;     /* The fraction carried past `base_ns`. */
	uint64_t last_ticks;   /* The counter reading `base_ns` belongs to. */
	uint64_t multiplier;
	uint32_t shift;
};

/* `mode`: do not use this page. Ask instead — `WIZ_OP_UPTIME` through the ring
 * is always reachable, which is what makes revocation possible at all. */
#define WIZ_TIME_MODE_NONE    0u
/* `mode`: the counter is good, and the conversion below is its. */
#define WIZ_TIME_MODE_COUNTER 1u

/* The reading, once version and page size are accepted and a stable copy of
 * the struct above has been taken. The fenced counter read belongs inside the
 * sequence window; the intermediate is 128-bit and the final sum saturates.
 *
 *   ticks = fenced_rdtsc() - c.last_ticks;
 *   wide  = (u128)c.fraction + (u128)ticks * c.multiplier;
 *   delta = wide >> c.shift;
 *   ns    = saturating_add(c.base_ns, delta);
 */

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
	WIZ_CALL_EXIT = 0,
	WIZ_CALL_RING_ENTER = 1,    /* Serve what has been submitted. */
	WIZ_CALL_TCB_SET = 2,       /* Point `fs:` at thread-local storage. */
	WIZ_CALL_RING_WAIT = 3,     /* Wait for an operation in flight (D-164). */
};

/* `WIZ_CALL_CLOCK_GET` is gone, and its absence is the point: the time is a
 * page, not a call (D-166). A libc reads `WIZ_TIME_ADDRESS`. */

/* Errors, as Linux numbers them, because that is what `abis/linux` brings. */
typedef int wiz_errno_t;

/* `sys_tcb_set` takes a base below WIZ_USER_END and refuses anything else:
 * writing a non-canonical value to FS_BASE is a fault in *kernel* context,
 * which would be a program crashing the handler rather than itself (D-118). */
#define WIZ_USER_END ((uint64_t)0x0000800000000000ull)

#define WIZ_ENOSYS 38
#define WIZ_EINVAL 22

/* ------------------------------------------------------------------ *
 * Signals
 * ------------------------------------------------------------------ */

/* D-80 requires signals to share the completion plane rather than introduce
 * an in-kernel signal frame. Version 1 has no unsolicited-event discriminator,
 * however: completions have no opcode and every token belongs to a submission.
 * No signal wire value exists until that representation and its first caller
 * are designed together (D-181). */

#ifdef __cplusplus
}
#endif

#endif /* WIZARD_ABI_H */
