#![expect(
    unsafe_code,
    reason = "the ring 3 runtime: entry, call gate, ring access and probes"
)]
#![no_std]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering, compiler_fence};

use native_abi::ring::{
    COMPLETION_BYTES, COMPLETIONS, COMPLETIONS_AT, Completion, ENTRIES_AT, SUBMISSION_BYTES,
    SUBMISSIONS, Submission, header,
};
use native_abi::{Call, address, entry};

const ALIGNMENT_CHECK: u64 = 1 << 18;

const NESTED_TASK: u64 = 1 << 14;

const NONCANONICAL: u64 = 0x0000_8000_0000_0000;

const KERNEL_HALF: u64 = 0xffff_8000_0000_0000;

const PANICKED: u64 = u64::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub version: u64,
    pub argument: u64,
    pub bootstrap_handle: u64,
    pub operand: u64,
}

impl Entry {
    /// # Safety
    ///
    /// `block` must be the stack pointer the kernel entered the program with, read before
    /// anything is pushed.
    pub unsafe fn read(block: *const u64) -> Self {
        let word = |offset: usize| {
            // SAFETY: the caller passes the entry stack pointer, under which the kernel wrote the
            // aligned, initialized entry block that `offset` indexes.
            unsafe { block.wrapping_byte_add(offset).read() }
        };
        Self {
            version: word(entry::offset::VERSION),
            argument: word(entry::offset::ARGUMENT),
            bootstrap_handle: word(entry::offset::BOOTSTRAP_HANDLE),
            operand: word(entry::offset::OPERAND),
        }
    }
}

#[macro_export]
macro_rules! entry {
    ($main:path) => {
        const __WITNESS_MAIN: fn(&$crate::Entry) -> u64 = $main;

        // `allow`, since `expect` never fires in a macro; the caller must `deny`, not `forbid`.
        #[allow(unsafe_code, reason = "witness-rt code expanding under the caller's deny")]
        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        extern "C" fn _start() -> ! {
            ::core::arch::naked_asm!(
                "mov rdi, rsp",
                "call {start}",
                "ud2",
                start = sym __witness_start,
            )
        }

        #[allow(unsafe_code, reason = "witness-rt code expanding under the caller's deny")]
        extern "C" fn __witness_start(block: *const u64) -> ! {
            // SAFETY: `_start` passes the stack pointer it was entered with and pushed nothing.
            let entry = unsafe { $crate::Entry::read(block) };
            $crate::exit(__WITNESS_MAIN(&entry))
        }
    };
}

// `AC` is set only across the gate, so trap entry must establish SMAP and no Rust access runs
// under alignment checking.
pub fn call(call: Call, argument: u64) -> u64 {
    let answer;
    // SAFETY: `int 0x80` enters the kernel, which preserves every register but `rax` and writes
    // only memory it shares with the program; the flags are restored around it.
    unsafe {
        asm!(
            "pushfq",
            "or qword ptr [rsp], {set}",
            "popfq",
            "int 0x80",
            "pushfq",
            "and qword ptr [rsp], {clear}",
            "popfq",
            set = const ALIGNMENT_CHECK,
            clear = const !ALIGNMENT_CHECK.cast_signed(),
            inout("rax") call.number() => answer,
            in("rdi") argument,
        );
    }
    answer
}

pub fn exit(status: u64) -> ! {
    call(Call::Exit, status);
    fault_with(Call::Exit.number())
}

pub fn fault_with(marker: u64) -> ! {
    // SAFETY: `ud2` raises `#UD`, which ends the program; nothing after it runs.
    unsafe { asm!("ud2", in("rax") marker, options(noreturn, nomem, nostack)) }
}

// Only a kernel-half target, whose fetch from ring 3 always faults, so no program code is reached.
pub fn jump_to_kernel(target: u64) -> ! {
    if target < KERNEL_HALF {
        fault_with(target);
    }
    // SAFETY: a ring 3 fetch from the kernel half faults and ends the program.
    unsafe { asm!("jmp rax", in("rax") target, options(noreturn, nomem, nostack)) }
}

// `rdtsc` with the marker in `rax`, so a trapped read leaves the marker for the fault record.
pub fn read_counter_with(marker: u64) -> ! {
    // SAFETY: `rdtsc` either traps, which ends the program, or returns into `ud2`, which does.
    unsafe {
        asm!(
            "rdtsc",
            "ud2",
            in("rax") marker,
            options(noreturn, nomem, nostack),
        )
    }
}

pub fn enter_with_noncanonical_stack(call: Call) -> ! {
    // SAFETY: the kernel refuses to return to a noncanonical stack and ends the program; were it to
    // return, `ud2` faults on that stack and ends it too.
    unsafe {
        asm!(
            "mov rsp, {stack}",
            "int 0x80",
            "ud2",
            stack = in(reg) NONCANONICAL,
            in("rax") call.number(),
            options(noreturn),
        )
    }
}

pub fn enter_with_nested_task(call: Call) -> ! {
    // SAFETY: the kernel refuses to return with `RFLAGS.NT` set and ends the program; were it to
    // return, `ud2` ends it.
    unsafe {
        asm!(
            "pushfq",
            "or qword ptr [rsp], {nested}",
            "popfq",
            "int 0x80",
            "ud2",
            nested = const NESTED_TASK,
            in("rax") call.number(),
            options(noreturn),
        )
    }
}

static RING_TAKEN: AtomicBool = AtomicBool::new(false);

// The kernel maps the ring at `address::RING` in every program; `take` hands out one view.
#[derive(Debug)]
pub struct Ring {
    base: usize,
}

impl Ring {
    pub fn take() -> Option<Self> {
        (!RING_TAKEN.swap(true, Ordering::AcqRel)).then_some(Self {
            base: address::RING as usize,
        })
    }

    fn at<T>(&self, offset: usize) -> *mut T {
        ptr::with_exposed_provenance_mut(self.base + offset)
    }

    fn load(&self, offset: usize) -> u32 {
        // SAFETY: the ring is mapped for the program's life, and `offset` names an aligned header
        // word inside its first page.
        unsafe { self.at::<u32>(offset).read_volatile() }
    }

    fn store(&mut self, offset: usize, value: u32) {
        // SAFETY: as in `load`; the program owns this index word by the ring protocol.
        unsafe { self.at::<u32>(offset).write_volatile(value) }
    }

    pub fn version(&self) -> u32 {
        self.load(header::VERSION)
    }

    pub fn submit(&mut self, index: u32, submission: Submission) {
        let slot = (index % SUBMISSIONS) as usize;
        let entry = self.at::<[u8; SUBMISSION_BYTES]>(ENTRIES_AT + slot * SUBMISSION_BYTES);
        // SAFETY: the slot lies inside the mapped entry page, aligned to its entry size.
        unsafe { entry.write_volatile(submission.encode()) }
    }

    // x86 keeps stores in order; the fence keeps the compiler from moving the entry after this.
    pub fn publish_submission_tail(&mut self, tail: u32) {
        compiler_fence(Ordering::Release);
        self.store(header::SUBMISSION_TAIL, tail);
    }

    pub fn completion_tail(&self) -> u32 {
        let tail = self.load(header::COMPLETION_TAIL);
        compiler_fence(Ordering::Acquire);
        tail
    }

    pub fn completion(&self, index: u32) -> Completion {
        let slot = (index % COMPLETIONS) as usize;
        let entry = self.at::<[u8; COMPLETION_BYTES]>(COMPLETIONS_AT + slot * COMPLETION_BYTES);
        // SAFETY: the slot lies inside the mapped entry page, aligned to its entry size.
        Completion::decode(&unsafe { entry.read_volatile() })
    }

    pub fn release_completion_head(&mut self, head: u32) {
        compiler_fence(Ordering::Release);
        self.store(header::COMPLETION_HEAD, head);
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    fault_with(PANICKED)
}
