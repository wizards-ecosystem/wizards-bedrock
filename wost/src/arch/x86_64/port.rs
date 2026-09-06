//! Typed x86 I/O port access.
//!
//! Ports are handed out by [`claim`] and [`claim_block`], which record every
//! claim in a registry so exclusivity is checked rather than merely promised.
//! Reads and writes are then ordinary safe calls, so a driver built on this
//! needs no `unsafe` of its own.
//!
//! A claim covers the ports an access *reaches*, not the one it names: an
//! `out dx, eax` to port `n` drives `n..n + 4`, so a `Port<u32>` reserves four.
//! The bookkeeping is `port_space::PortSpace`, where it is host-tested; this
//! module is the lock around it and the instructions underneath it.
//!
//! **A fixed-driver claim is permanent, so a register set is claimed as one
//! transaction.** [`PortBlock`] and [`Port`] have no `Drop`, which is right for
//! a driver that owns its registers for the life of the kernel. It also means
//! a caller must not take one member and then strand it when a later member is
//! unavailable. [`claim_pair`] answers that for i8042 under one lock;
//! [`claim_block_pair`] does the same for the PIC (D-138).

use core::arch::asm;
use core::marker::PhantomData;
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, Ordering};

use port_space::PortSpace;

use crate::panic::PanicToken;
use crate::sync::SpinLock;

/// Which ports are spoken for. Fixed claims are permanent; only unique dynamic
/// span capabilities can return a transient lease.
static CLAIMED: SpinLock<PortSpace> = SpinLock::new(PortSpace::new());

#[cfg(feature = "selftest")]
static REFUSE_SPAN_RELEASE: AtomicBool = AtomicBool::new(false);

/// Record `len` ports from `number` as claimed, all or nothing.
fn reserve(number: u16, len: usize) -> bool {
    CLAIMED.lock_leaf().reserve(number as usize, len)
}

/// Claim exclusive access to port `number` and the ports a `T`-wide access
/// reaches beyond it.
///
/// Returns `None` if any of them is already claimed, or if the access would
/// run past the end of I/O space.
pub fn claim<T: PortValue>(number: u16) -> Option<Port<T>> {
    reserve(number, size_of::<T>()).then(|| {
        // SAFETY: the registry has just recorded every port a `T`-wide access
        // at `number` drives, and no claim is ever released, so no other
        // `Port` or `PortBlock` can cover any of them.
        unsafe { Port::new(number) }
    })
}

/// Claim two ports at once, or neither.
///
/// The answer to the trap named above: a claim is permanent, so a caller that
/// takes one port and then fails to take the second has foreclosed the first
/// for the life of the boot, and the thing that needed both can never be
/// built. Chaining two [`claim`]s cannot avoid it — the check and the record
/// are two moments and there is no undo — so the question is asked once,
/// against one bitmap, under one lock (D-138).
///
/// `None` if either port is already claimed, if the two overlap, or if either
/// runs past the end of I/O space.
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
    // SAFETY: the registry has just recorded every port both accesses drive,
    // as one all-or-none transaction, and no claim is ever released — so no
    // other `Port` or `PortBlock` can cover any of them.
    let first = unsafe { Port::new(first) };
    // SAFETY: as above; the same reservation covered this port, and
    // `reserve_all` refused the pair had the two overlapped.
    let second = unsafe { Port::new(second) };
    Some((first, second))
}

/// Claim several machine-described runs as one transaction.
///
/// A PCI function may decode several I/O BARs. Claiming them one at a time
/// would strand the early runs if a later one were already owned. A transient
/// owner may later return this exact set through [`release_spans`].
///
/// `None`, with no run recorded, if the request is empty, has more than `N`
/// spans, or any span is empty, overlapping, out of range, or already owned.
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
/// The address-based bitmap operation stays behind the unique [`PortSpan`]
/// values returned by [`claim_spans`]. Consuming the whole array prevents a
/// stale safe access handle from surviving a successful release (D-199).
///
/// An empty array succeeds without touching the registry. A refusal returns
/// the unchanged capabilities for an exact retry.
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

/// A claimed run of ports of a length read from the machine.
///
/// Carries its length so every access can be bounded by it. The alternative —
/// a base and a caller that remembers — is the arrangement `mmio_range` exists
/// to refuse on the memory side, and there is no reason the port side should
/// be looser.
#[derive(Debug)]
pub struct PortSpan {
    base: u16,
    len: usize,
}

/// A refused all-or-none span return with every capability unchanged.
#[derive(Debug)]
pub struct SpanReleaseFailure<const N: usize> {
    /// The exact spans still reserved in the registry.
    pub spans: [Option<PortSpan>; N],
}

/// Refuse the next dynamic span release before the registry changes.
#[cfg(feature = "selftest")]
pub fn refuse_span_release_for_test() {
    REFUSE_SPAN_RELEASE.store(true, Ordering::Release);
}

/// Claim exclusive access to the `N` consecutive ports starting at `base`.
///
/// Returns `None` if any of them is already claimed, or if the block would run
/// past the end of I/O space.
pub fn claim_block<const N: usize>(base: u16) -> Option<PortBlock<N>> {
    reserve(base, N).then(|| {
        // SAFETY: the registry has just recorded every port in the block, and
        // no claim is ever released, so nothing else can cover them.
        unsafe { PortBlock::new(base) }
    })
}

/// Claim two blocks at once, or neither.
///
/// [`claim_pair`]'s reasoning, for the callers that drive register windows
/// rather than single ports (D-138).
///
/// `None` if either block is already claimed, if the two overlap, or if either
/// runs past the end of I/O space.
pub fn claim_block_pair<const N: usize, const M: usize>(
    first: u16,
    second: u16,
) -> Option<(PortBlock<N>, PortBlock<M>)> {
    let spans = [(first as usize, N), (second as usize, M)];
    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port in both blocks, as one
    // all-or-none transaction, and no claim is ever released — so nothing else
    // can cover any of them.
    let first = unsafe { PortBlock::new(first) };
    // SAFETY: as above; `reserve_all` refused the pair had the two overlapped.
    let second = unsafe { PortBlock::new(second) };
    Some((first, second))
}

/// Reclaim the registry's lock on the terminal panic path.
///
/// Called by [`panic::reclaim`](crate::panic::reclaim) rather than directly:
/// the panic path has to be able to claim a port to report through, and the
/// code it interrupted may have been part-way through a claim of its own.
pub(crate) fn force_unlock(panic: &PanicToken) {
    CLAIMED.force_unlock(panic);
}

mod sealed {
    pub trait Sealed {}
}

/// A width of value that can be transferred over an I/O port.
///
/// Sealed: the x86 port instructions exist only for 8, 16, and 32 bits, and
/// the only implementors are the primitives of exactly those widths. That is
/// what lets `size_of::<T>()` stand for the number of ports an access drives,
/// which is how [`claim`] knows how much to reserve.
pub trait PortValue: sealed::Sealed + Copy {
    /// Read one value from `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`. Reads from device ports are not necessarily
    /// pure — many advance a FIFO or clear a status latch as a side effect.
    unsafe fn port_read(port: u16) -> Self;

    /// Write one value to `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`. A port write drives hardware directly and
    /// can have arbitrary effects on the machine.
    unsafe fn port_write(port: u16, value: Self);
}

impl sealed::Sealed for u8 {}

impl PortValue for u8 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u8;
        // SAFETY: `in al, dx` reads one byte from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, al` writes one byte to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u16 {}

impl PortValue for u16 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u16;
        // SAFETY: `in ax, dx` reads one word from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, ax` writes one word to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
        unsafe {
            asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u32 {}

impl PortValue for u32 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u32;
        // SAFETY: `in eax, dx` reads one dword from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, eax` writes one dword to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
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
    /// Claim `number`.
    ///
    /// Deliberately not public: [`claim`] is the only door, so a handle cannot
    /// exist without the registry knowing about it.
    ///
    /// # Safety
    ///
    /// No other live `Port` or [`PortBlock`] may cover any of the
    /// `size_of::<T>()` ports starting at `number` — the span a `T`-wide
    /// access drives — and the caller must be entitled to drive whatever
    /// device answers there.
    const unsafe fn new(number: u16) -> Self {
        Self {
            number,
            value: PhantomData,
        }
    }

    /// Read one value from the port.
    pub fn read(&mut self) -> T {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to
        // `self.number`, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_read(self.number) }
    }

    /// Write one value to the port.
    pub fn write(&mut self, value: T) {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to
        // `self.number`, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_write(self.number, value) };
    }
}

/// Exclusive access to `N` consecutive I/O ports — a device's register file.
///
/// Accesses are checked against `N` by the span they reach, not by the offset
/// they name, so a driver indexes its registers by name without being able to
/// walk off the end into another device — including with a `u16` or `u32`
/// access that starts inside the block and would finish outside it.
#[derive(Debug)]
pub struct PortBlock<const N: usize> {
    base: u16,
}

impl<const N: usize> PortBlock<N> {
    /// Claim the `N` ports starting at `base`.
    ///
    /// Deliberately not public: [`claim_block`] is the only door, so a handle
    /// cannot exist without the registry knowing about it.
    ///
    /// # Safety
    ///
    /// No other live [`Port`] or `PortBlock` may overlap `base..base + N`, and
    /// the caller must be entitled to drive the device that answers there.
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
        // SAFETY: `port_for` confirmed the whole access lies inside the block
        // this handle exclusively owns, and `&mut self` rules out a concurrent
        // access.
        unsafe { <T as PortValue>::port_read(self.port_for::<T>(offset)) }
    }

    /// Write register `offset`.
    ///
    /// # Panics
    ///
    /// Panics unless the whole `T`-wide access lies within the block.
    pub fn write<T: PortValue>(&mut self, offset: usize, value: T) {
        // SAFETY: `port_for` confirmed the whole access lies inside the block
        // this handle exclusively owns, and `&mut self` rules out a concurrent
        // access.
        unsafe { <T as PortValue>::port_write(self.port_for::<T>(offset), value) };
    }

    /// The port a `T`-wide access at `offset` starts on, having confirmed that
    /// every port it goes on to drive is inside the block.
    fn port_for<T: PortValue>(&self, offset: usize) -> u16 {
        let width = size_of::<T>();
        assert!(
            offset.checked_add(width).is_some_and(|end| end <= N),
            "a {width}-byte access at register {offset} leaves a {N}-port block"
        );
        // Cannot overflow: `new` bounded `base + N` to the end of I/O space and
        // the assertion above bounds `offset` below `N`.
        self.base + offset as u16
    }
}
