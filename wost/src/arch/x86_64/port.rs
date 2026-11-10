//! Typed x86 I/O port access whose every claim a registry records, so exclusivity is checked.
//!
//! A claim covers every port an access reaches: `out dx, eax` to port `n` drives `n..n + 4`.
//! A fixed-driver claim is permanent ([`PortBlock`] and [`Port`] have no `Drop`), so a register
//! set is claimed as one transaction: a caller must not take one member and then strand it when a
//! later member is unavailable. [`claim_pair`] answers that for i8042 under one lock;
//! [`claim_block_pair`] does the same for the PIC.

use core::arch::asm;
use core::marker::PhantomData;
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, Ordering};

use port_space::PortSpace;

use crate::panic::PanicToken;
use crate::sync::SpinLock;

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
use selftest::REFUSE_SPAN_RELEASE;
#[cfg(feature = "selftest")]
pub use selftest::{is_claimed, refuse_span_release_for_test};

/// Which ports are spoken for; only unique dynamic span capabilities can return a lease.
static CLAIMED: SpinLock<PortSpace> = SpinLock::new(PortSpace::new());

/// Record `len` ports from `number` as claimed, all or nothing.
fn reserve(number: u16, len: usize) -> bool {
    CLAIMED.lock_leaf().reserve(number as usize, len)
}

/// Claim port `number` and the ports a `T`-wide access reaches beyond it.
///
/// `None` if any of them is already claimed, or if the access would run past the end of I/O space.
pub fn claim<T: PortValue>(number: u16) -> Option<Port<T>> {
    reserve(number, size_of::<T>()).then(|| {
        // SAFETY: the registry has just recorded every port a `T`-wide access at `number` drives,
        // and no claim is ever released, so no other `Port` or `PortBlock` can cover any of them.
        unsafe { Port::new(number) }
    })
}

/// Claim two ports at once, or neither.
///
/// Two chained [`claim`]s check and record at two moments with no undo, so the pair is asked once.
/// `None` if either port is already claimed, if the two overlap, or if either runs past the end
/// of I/O space.
pub fn claim_pair<T: PortValue, U: PortValue>(
    first: u16,
    second: u16,
) -> Option<(Port<T>, Port<U>)> {
    let spans = [
        (first as usize, size_of::<T>()),
        (second as usize, size_of::<U>()),
    ];
    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive, all or none, and no
    // claim is ever released, so no other `Port` or `PortBlock` can cover any of them.
    let first = unsafe { Port::new(first) };
    // SAFETY: as above; the same reservation covered this port, and `reserve_all` refused the
    // pair had the two overlapped.
    let second = unsafe { Port::new(second) };
    Some((first, second))
}

/// Claim several machine-described runs, such as a function's I/O BARs, as one transaction.
///
/// A transient owner may later return this exact set through [`release_spans`]. `None`, with no
/// run recorded, if the request is empty, has more than `N` spans, or any span is empty,
/// overlapping, out of range, or already owned.
pub fn claim_spans<const N: usize>(spans: &[(u16, usize)]) -> Option<[Option<PortSpan>; N]> {
    if spans.is_empty() || spans.len() > N {
        return None;
    }
    let mut requested = [(0usize, 0usize); N];
    for (slot, &(base, len)) in requested.iter_mut().zip(spans) {
        *slot = (base as usize, len);
    }
    if !CLAIMED.lock_leaf().reserve_all(&requested[..spans.len()]) {
        return None;
    }

    let mut claimed = [const { None }; N];
    for (slot, &(base, len)) in claimed.iter_mut().zip(spans) {
        *slot = Some(PortSpan { base, len });
    }
    Some(claimed)
}

/// Return one transient device lease's dynamic spans, all or none.
///
/// Consuming the whole array keeps a stale access handle from surviving a successful release.
/// An empty array succeeds without touching the registry; a refusal returns the unchanged
/// capabilities for an exact retry.
pub fn release_spans<const N: usize>(
    spans: [Option<PortSpan>; N],
) -> Result<(), SpanReleaseFailure<N>> {
    #[cfg(feature = "selftest")]
    if REFUSE_SPAN_RELEASE.swap(false, Ordering::AcqRel) {
        return Err(SpanReleaseFailure { spans });
    }
    let mut requested = [(0usize, 0usize); N];
    let mut count = 0;
    for span in spans.iter().flatten() {
        requested[count] = (span.base as usize, span.len);
        count += 1;
    }
    if count == 0 || CLAIMED.lock_leaf().release_all(&requested[..count]) {
        Ok(())
    } else {
        Err(SpanReleaseFailure { spans })
    }
}

/// A claimed run of ports of a length read from the machine, carried so every access is bounded.
#[derive(Debug)]
pub struct PortSpan {
    base: u16,
    len: usize,
}

/// A refused all-or-none span return with every capability unchanged.
#[derive(Debug)]
#[must_use = "a refused span release returns every reserved span"]
pub struct SpanReleaseFailure<const N: usize> {
    /// The exact spans still reserved in the registry.
    pub spans: [Option<PortSpan>; N],
}

/// Claim the `N` consecutive ports starting at `base`.
///
/// `None` if any of them is already claimed, or if the block would run past the end of I/O space.
pub fn claim_block<const N: usize>(base: u16) -> Option<PortBlock<N>> {
    reserve(base, N).then(|| {
        // SAFETY: the registry has just recorded every port in the block, and no claim is ever
        // released, so nothing else can cover them.
        unsafe { PortBlock::new(base) }
    })
}

/// Claim two register windows at once, or neither, for [`claim_pair`]'s reason.
///
/// `None` if either block is already claimed, if the two overlap, or if either runs past the end
/// of I/O space.
pub fn claim_block_pair<const N: usize, const M: usize>(
    first: u16,
    second: u16,
) -> Option<(PortBlock<N>, PortBlock<M>)> {
    let spans = [(first as usize, N), (second as usize, M)];
    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port in both blocks, all or none, and no claim
    // is ever released, so nothing else can cover any of them.
    let first = unsafe { PortBlock::new(first) };
    // SAFETY: as above; `reserve_all` refused the pair had the two overlapped.
    let second = unsafe { PortBlock::new(second) };
    Some((first, second))
}

/// The QEMU debug-exit device, asked for by name so no component claims a raw port it does not own.
pub fn debug_exit() -> Option<Port<u32>> {
    claim(0xF4)
}

/// Both legacy 8237 DMA controllers, DMA1 at `0x00..0x10` and DMA2 at `0xC0..0xE0`, as one claim.
pub fn isa_dma_controllers() -> Option<(PortBlock<16>, PortBlock<32>)> {
    claim_block_pair(0x00, 0xC0)
}

/// The 8042 keyboard controller's data and command ports (0x60 and 0x64), as one claim.
pub fn i8042() -> Option<(Port<u8>, Port<u8>)> {
    claim_pair(0x60, 0x64)
}

/// The 8254 interval timer's four registers (0x40 through 0x43).
pub fn pit() -> Option<PortBlock<4>> {
    claim_block(0x40)
}

/// The first serial controller's eight registers (0x3F8 through 0x3FF).
pub fn com1() -> Option<PortBlock<8>> {
    claim_block(0x3F8)
}

/// The legacy PCI configuration ports (0xCF8 and 0xCFC), the one door to configuration space.
///
/// A degraded boot with no bus claims it to prove nothing else did.
pub fn pci_configuration() -> Option<(Port<u32>, Port<u32>)> {
    claim_pair(0x0CF8, 0x0CFC)
}

/// Reclaim the registry's lock for [`panic::reclaim`](crate::panic::reclaim), since the code the
/// panic interrupted may have been part-way through a claim and the panic path must claim a port.
pub(crate) fn force_unlock(panic: &PanicToken) {
    CLAIMED.force_unlock(panic);
}

mod sealed {
    pub trait Sealed {}
}

/// A width of value that can be transferred over an I/O port.
///
/// Sealed to `u8`, `u16` and `u32`, the only port instruction widths, so `size_of::<T>()` is the
/// number of ports an access drives and [`claim`] knows how much to reserve.
pub trait PortValue: sealed::Sealed + Copy {
    /// Read one value from `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`, and want the read's side effects: many advance a FIFO or clear
    /// a status latch.
    unsafe fn port_read(port: u16) -> Self;

    /// Write one value to `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`; a port write can have arbitrary effects on the machine.
    unsafe fn port_write(port: u16, value: Self);
}

impl sealed::Sealed for u8 {}

impl PortValue for u8 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u8;
        // SAFETY: `in al, dx` reads one byte from the port in `dx`; the caller guarantees the port
        // is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, al` writes one byte to the port in `dx`; the caller guarantees the port
        // is theirs to drive.
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u16 {}

impl PortValue for u16 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u16;
        // SAFETY: `in ax, dx` reads one word from the port in `dx`; the caller guarantees the port
        // is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, ax` writes one word to the port in `dx`; the caller guarantees the port
        // is theirs to drive.
        unsafe {
            asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u32 {}

impl PortValue for u32 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u32;
        // SAFETY: `in eax, dx` reads one dword from the port in `dx`; the caller guarantees the
        // port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, eax` writes one dword to the port in `dx`; the caller guarantees the
        // port is theirs to drive.
        unsafe {
            asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

/// Exclusive access to a single I/O port.
#[derive(Debug)]
pub struct Port<T: PortValue> {
    number: u16,
    value: PhantomData<T>,
}

impl<T: PortValue> Port<T> {
    /// Private so [`claim`] is the only door and the registry knows every handle.
    ///
    /// # Safety
    ///
    /// No other live `Port` or [`PortBlock`] may cover any of the `size_of::<T>()` ports a `T`-wide
    /// access at `number` drives, and the caller must be entitled to drive the device there.
    const unsafe fn new(number: u16) -> Self {
        Self {
            number,
            value: PhantomData,
        }
    }

    pub fn read(&mut self) -> T {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to `self.number`, and
        // `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_read(self.number) }
    }

    pub fn write(&mut self, value: T) {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to `self.number`, and
        // `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_write(self.number, value) };
    }
}

/// Exclusive access to a device's `N` consecutive register ports.
///
/// Each access is checked by the whole span it reaches, so a `u16` or `u32` access starting inside
/// the block cannot finish in another device.
#[derive(Debug)]
pub struct PortBlock<const N: usize> {
    base: u16,
}

impl<const N: usize> PortBlock<N> {
    /// Private so [`claim_block`] is the only door and the registry knows every handle.
    ///
    /// # Safety
    ///
    /// No other live [`Port`] or `PortBlock` may overlap `base..base + N`, and the caller must be
    /// entitled to drive the device that answers there.
    ///
    /// # Panics
    ///
    /// Panics if the block is empty or would run past port `0xFFFF`.
    const unsafe fn new(base: u16) -> Self {
        assert!(N > 0, "a port block covers at least one port");
        assert!(
            base as usize + N <= port_space::PORT_COUNT,
            "port block runs past the end of I/O space"
        );
        Self { base }
    }

    /// Read register `offset`.
    ///
    /// # Panics
    ///
    /// Panics unless the whole `T`-wide access lies within the block.
    pub fn read<T: PortValue>(&mut self, offset: usize) -> T {
        // SAFETY: `port_for` confirmed the whole access lies inside the block this handle
        // exclusively owns, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_read(self.port_for::<T>(offset)) }
    }

    /// Write register `offset`.
    ///
    /// # Panics
    ///
    /// Panics unless the whole `T`-wide access lies within the block.
    pub fn write<T: PortValue>(&mut self, offset: usize, value: T) {
        // SAFETY: `port_for` confirmed the whole access lies inside the block this handle
        // exclusively owns, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_write(self.port_for::<T>(offset), value) };
    }

    /// The port a `T`-wide access at `offset` starts on, once every port it drives is in the block.
    fn port_for<T: PortValue>(&self, offset: usize) -> u16 {
        let width = size_of::<T>();
        assert!(
            offset.checked_add(width).is_some_and(|end| end <= N),
            "a {width}-byte access at register {offset} leaves a {N}-port block"
        );
        // Cannot overflow: `new` bounded `base + N` to I/O space and the assertion bounds `offset`.
        self.base + offset as u16
    }
}
