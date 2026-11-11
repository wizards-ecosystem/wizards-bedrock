use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use wost::arch::current::cpu;
use wost::arch::current::paging;
use wost::arch::current::{interrupt, port, segment, xstate};
use wost::boot::{FramebufferInfo, Rgb};
use wost::sync::{Once, SpinLock, SpinLockWitness};
use x86_paging::Flags;
use x86_tables::exception;

use super::clocks::spins_until;
use super::threads::context;
use crate::arch::current::trap::Record;
use crate::arch::current::uart;
use crate::arch::current::vector;
use crate::iommu;
use crate::{dev, input, thread, time};

pub(super) fn the_editing_keys_move_the_cursor() -> bool {
    let (Some(backspace), Some(newline)) = (
        ps2_keys::us::char_for(ps2_keys::Key::Backspace, ps2_keys::Shift::new()),
        ps2_keys::us::char_for(ps2_keys::Key::Enter, ps2_keys::Shift::new()),
    ) else {
        return false;
    };

    let Some(((start_x, start_y), (glyph_width, line_height))) = dev::console_cursor() else {
        return false;
    };

    dev::with_console(|console| {
        let _ = console.write_char('X');
    });
    let Some(((typed_x, typed_y), _)) = dev::console_cursor() else {
        return false;
    };
    let advanced = typed_x == start_x + glyph_width && typed_y == start_y;

    dev::with_console(|console| {
        let _ = console.write_char(backspace);
    });
    let Some(((erased_x, erased_y), _)) = dev::console_cursor() else {
        return false;
    };
    let erased = erased_x == start_x && erased_y == start_y;

    // From a column past the margin, so the return to it is observable.
    dev::with_console(|console| {
        let _ = console.write_char('X');
        let _ = console.write_char(newline);
    });
    let Some(((home_x, home_y), _)) = dev::console_cursor() else {
        return false;
    };
    // The margin, not `home_x <= start_x`, which holds for every reset; the margin is where
    // `erase_previous` refuses to go below.
    let started_a_line =
        home_x == crate::console::MARGIN && home_x <= typed_x && home_y == start_y + line_height;

    advanced && erased && started_a_line
}

pub(super) fn the_framebuffer_is_write_combining() -> bool {
    cpu::write_combining_is_enabled()
        && dev::framebuffer_address()
            .and_then(paging::translate)
            .is_some_and(|(_, flags)| {
                flags.contains(Flags::WRITE_THROUGH)
                    && !flags.contains(Flags::NO_CACHE)
                    && !flags.is_write_execute()
            })
}

pub(super) fn default_deny_interrupt_remapping_guards_pci() -> bool {
    iommu::interrupt_boundary_is_deny_all_for_test()
        && iommu::interrupt_boundary_active()
        && !input::is_interrupt_driven()
}

pub(super) fn the_io_apic_owner_was_returned_after_masked_rollback() -> bool {
    crate::keyboard_routing_rollback_is_exact()
        && dev::io_apic_base().is_none()
        && dev::routing_of(dev::keyboard_gsi()).is_none()
}

pub(super) fn keyboard_input_is_polled_without_legacy_interrupts() -> bool {
    !input::is_interrupt_driven()
        && input::interrupt_count() == 0
        && input::poller_started_for_test()
        && input::poller_passes() > 0
}

pub(super) fn a_scroll_moves_one_visible_cursor() -> bool {
    const AT_X: usize = crate::console::MARGIN;

    let Some(((_, mut cursor_y), (_, line_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(geometry) = dev::console_geometry() else {
        return false;
    };

    let mut scrolled_from = None;
    for _ in 0..=geometry.height / line_height.max(1) {
        dev::with_console(|console| console.write_line(""));
        let Some(((.., next_y), _)) = dev::console_cursor() else {
            return false;
        };
        if next_y == cursor_y {
            scrolled_from = Some(cursor_y);
            break;
        }
        cursor_y = next_y;
    }

    // The cursor underline moves one pixel above its prior origin. Blank input
    // leaves no other legitimate foreground mark there.
    let Some(probe_y) = scrolled_from.and_then(|y| y.checked_sub(1)) else {
        return false;
    };
    let Some((old_pixel, foreground, _)) = dev::console_probe(AT_X, probe_y) else {
        return false;
    };
    let Some(((cursor_x, cursor_y), (_, cursor_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(cursor_y) = cursor_y.checked_add(cursor_height - 1) else {
        return false;
    };
    let Some((cursor_pixel, cursor_foreground, _)) = dev::console_probe(cursor_x, cursor_y) else {
        return false;
    };
    old_pixel != foreground && cursor_pixel == cursor_foreground
}

pub(super) fn a_scroll_does_not_block_interrupts() -> bool {
    const DELIVERY_ATTEMPTS: u32 = 1_000_000;
    const EXPECTED_EXPIRIES: u64 = 1;

    let Some((_, (_, line_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(geometry) = dev::console_geometry() else {
        return false;
    };
    let rows = geometry.height / line_height.max(1);

    let measured = (|| {
        let mut ready = false;
        for _ in 0..=rows {
            dev::with_console(|console| {
                ready = console.next_line_scrolls();
                if !ready {
                    console.write_line("");
                }
            });
            if ready {
                break;
            }
        }
        if !ready {
            return None;
        }

        let mut result = None;
        dev::with_console(|console| {
            let cursor = console.cursor().1;
            let excluded = interrupt::disable();
            let taken = time::expiries();
            time::arm_at(time::scheduler_now_ns());
            excluded.restore();
            let before = time::now_ns();
            console.write_line("");
            let elapsed = time::now_ns() - before;
            // Framebuffer MMIO can finish before an already-due virtual APIC timer is injected.
            // The observation stays inside the console guard: mutation 189's outer exclusion must
            // prevent this real delivery until after the result has been fixed as false.
            let delivered = spins_until(DELIVERY_ATTEMPTS, || {
                time::expiries() - taken >= EXPECTED_EXPIRIES
            });
            result = Some((elapsed, delivered, console.cursor().1 == cursor));
        });
        result
    })();
    thread::rearm_current_for_test();

    let Some((elapsed, delivered, scrolled)) = measured else {
        LONGEST_SCROLL.store(0, Ordering::Relaxed);
        return false;
    };
    LONGEST_SCROLL.store(elapsed as usize, Ordering::Relaxed);

    scrolled && elapsed > 0 && delivered
}

pub(super) static LONGEST_SCROLL: AtomicUsize = AtomicUsize::new(0);

pub(super) fn geometry_is_sane(geometry: FramebufferInfo) -> bool {
    let described = geometry.width > 0
        && geometry.height > 0
        && geometry.bytes_per_pixel > 0
        && geometry.pitch >= geometry.width * geometry.bytes_per_pixel;

    let last = (geometry.height - 1) * geometry.pitch + geometry.width * geometry.bytes_per_pixel;
    described && dev::framebuffer_bytes().is_some_and(|mapped| last <= mapped)
}

pub(super) fn channels_are_disjoint(geometry: FramebufferInfo) -> bool {
    let pack = |red, green, blue| geometry.format.pack(Rgb { red, green, blue });
    let (red, green, blue) = (
        pack(u8::MAX, 0, 0),
        pack(0, u8::MAX, 0),
        pack(0, 0, u8::MAX),
    );

    let disjoint = red != 0
        && green != 0
        && blue != 0
        && red & green == 0
        && green & blue == 0
        && red & blue == 0;

    // The three compose as the format says, which disjoint masks alone do not show: a `pack`
    // dropping a channel passes them.
    let white = pack(u8::MAX, u8::MAX, u8::MAX) == red | green | blue;
    let black = pack(0, 0, 0) == 0;

    disjoint && white && black
}

pub(super) fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none() && port::claim::<u8>(uart::COM1).is_none()
}

pub(super) fn a_save_stays_inside_its_area() -> bool {
    if !xstate::is_enabled() {
        // The bounds check has no instruction to exercise; user launch refuses.
        return xstate::bytes() == 0;
    }
    // The form is whatever the processor supports; what must hold on every
    // processor is that the area was sized for the form in use.
    xstate::save_stays_inside_its_area()
}

pub(super) fn the_fast_syscall_doors_are_shut_by_this_kernel() -> bool {
    cpu::drive_fast_syscall_shutdown_for_test()
}

pub(super) fn a_refused_pair_of_claims_strands_neither() -> bool {
    /// Free, and two below COM1 so it cannot collide with the serial block.
    const FREE: u16 = uart::COM1 - 4;

    // COM1 is the serial driver's, so any pair naming it must be refused.
    let refused = port::claim_pair::<u8, u8>(FREE, uart::COM1).is_none();

    // The free half is still free: chaining would have taken it on the way to failing.
    let untouched = port::claim::<u8>(FREE).is_some();

    // This also proves the first line was a real refusal and not a refusal of `FREE` itself, since
    // it was claimable a moment later.
    let taken_now = port::claim::<u8>(FREE).is_none();

    refused && untouched && taken_now
}

pub(super) fn a_claim_covers_the_width_it_drives() -> bool {
    const BELOW_COM1: u16 = uart::COM1 - 2;

    port::claim::<u32>(BELOW_COM1).is_none()
        && port::claim::<u8>(BELOW_COM1).is_some()
        && port::claim::<u8>(BELOW_COM1).is_none()
}

pub(super) fn lock_excludes() -> bool {
    let lock = SpinLock::new(0u32);
    let held = lock.lock_leaf();
    let refused = lock.try_lock_leaf().is_none();
    drop(held);
    refused && lock.try_lock_leaf().is_some()
}

pub(super) fn forgotten_lock_ownership_is_distinct_from_contention() -> bool {
    let lock = SpinLock::new(0u32);
    let mut context = context();
    let interrupts_before = interrupt::are_enabled();

    let held = lock.lock(&mut context);
    let ordinary =
        lock.witness_for_test() == SpinLockWitness::Contended && lock.try_lock_leaf().is_none();

    let recovery = held.forget_for_test();
    let forgotten =
        lock.witness_for_test() == SpinLockWitness::Forgotten && lock.try_lock_leaf().is_none();
    drop(recovery);

    let restored = lock.witness_for_test() == SpinLockWitness::Free
        && interrupt::are_enabled() == interrupts_before;
    let reacquired = lock.try_lock(&mut context);
    let available = reacquired.is_some();
    drop(reacquired);

    ordinary
        && forgotten
        && restored
        && available
        && lock.witness_for_test() == SpinLockWitness::Free
        && interrupt::are_enabled() == interrupts_before
}

pub(super) fn latch_admits_one() -> bool {
    let latch = Once::new();
    !latch.is_claimed() && latch.claim() && !latch.claim() && latch.is_claimed()
}

/// Every input line on both legacy controllers, disconnected.
pub(super) const MASKED: u8 = 0xFF;

pub(super) fn a_lock_excludes_interrupts() -> bool {
    let outer = SpinLock::new(0u32);
    let inner = SpinLock::new(0u32);
    let mut context = context();

    let before = interrupt::are_enabled();
    // The sanctioned order is one token-bearing outer lock and one inner leaf. Two leaves would
    // trip the leaf-guard detector and mask a later real violation.
    let outer_guard = outer.lock(&mut context);
    let excluded = !interrupt::are_enabled();

    let inner_guard = inner.lock_leaf();
    drop(inner_guard);
    let still_excluded = !interrupt::are_enabled();

    drop(outer_guard);
    before && excluded && still_excluded && interrupt::are_enabled()
}

// Interpret identity in the observed translation mode because a translator rewrites replies and
// scancodes alike.
pub(super) const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);

/// The same keyboard, behind a controller that translates.
pub(super) const TRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);

pub(super) fn keyboard_config_matches_its_routing() -> bool {
    const FIRST_IRQ: u8 = 1 << 0;
    const SECOND_IRQ: u8 = 1 << 1;
    const TRANSLATE: u8 = 1 << 6;

    let Some(byte) = input::controller_config() else {
        return false;
    };

    // Translation stays off: a controller quietly converting to set 1 is the classic keyboard that
    // types the wrong characters.
    let raw = byte & TRANSLATE == 0;
    // The second port is never enabled, since nothing is on it.
    let second_silent = byte & SECOND_IRQ == 0;
    let first_matches_routing = (byte & FIRST_IRQ != 0) == input::is_interrupt_driven();

    raw && second_silent && first_matches_routing
}

pub(super) fn ping_is_consistent(ping: Record) -> bool {
    /// `RFLAGS` bit 9.
    const INTERRUPT_FLAG: u64 = 1 << 9;

    ping.vector == u64::from(vector::PING.get())
        && ping.error_code == 0
        && ping.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && ping.flags & INTERRUPT_FLAG != 0
}

pub(super) fn frame_is_consistent(breakpoint: Record) -> bool {
    /// Bit 1 of the flags register reads as one on every x86 ever made.
    const RESERVED_FLAG: u64 = 1 << 1;

    breakpoint.vector == u64::from(exception::BREAKPOINT)
        && breakpoint.error_code == 0
        && breakpoint.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && breakpoint.stack_selector == u64::from(segment::KERNEL_DATA.bits())
        && breakpoint.flags & RESERVED_FLAG != 0
        && breakpoint.stack_pointer == breakpoint.rax
}

pub(super) fn frame_spans_the_saved_words(breakpoint: Record) -> bool {
    breakpoint
        .frame_address
        .checked_add(interrupt::SAVED_FRAME_BYTES)
        == Some(breakpoint.rax)
}
