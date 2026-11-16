use core::fmt;
use core::mem::ManuallyDrop;

use frame_space::{Access, ArenaIndex, ColourCounts, MAX_COLOURS, Owner, Reservation};
pub use frame_space::{ColourMask, FrameKind};

use super::direct::DirectSpan;
use super::frame::{frame_address, zero_frame};
use super::{FRAMES, Frames, table};
use crate::arch::current::paging::{DirectAccess, Flush, set_direct_access};
use crate::sync::ThreadContext;

const CAPACITY: usize = 256;

const FRAME_BYTES: usize = frame_space::FRAME_SIZE as usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaId {
    index: u32,
    generation: u32,
}

impl ArenaId {
    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    const fn record(self) -> ArenaIndex {
        ArenaIndex::new(self.index)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refused {
    NoMemory,
    NoNode,
    Stale,
    Mask,
    Busy,
    Twice,
}

impl fmt::Display for Refused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoMemory => "the reservation cannot cover the request",
            Self::NoNode => "every arena node is in use",
            Self::Stale => "the arena is gone or dying",
            Self::Mask => "the colour mask lies outside the parent's",
            Self::Busy => "the arena still has earmarks or frame tokens outstanding",
            Self::Twice => "the root arena already exists",
        })
    }
}

impl core::error::Error for Refused {}

#[derive(Debug)]
struct Node {
    generation: u32,
    live: bool,
    dying: bool,
    earmarked: usize,
    tokens: usize,
    cursor: usize,
    reservation: Reservation,
}

impl Node {
    const FREE: Self = Self {
        generation: 0,
        live: false,
        dying: false,
        earmarked: 0,
        tokens: 0,
        cursor: 0,
        reservation: Reservation::EMPTY,
    };
}

pub(super) struct Nodes {
    nodes: [Node; CAPACITY],
    root: bool,
}

impl Nodes {
    #[expect(
        clippy::large_stack_arrays,
        reason = "evaluated only in the allocator's static initializer, never on a stack"
    )]
    pub(super) const fn new() -> Self {
        Self {
            nodes: [const { Node::FREE }; CAPACITY],
            root: false,
        }
    }

    fn live(&mut self, arena: ArenaId) -> Option<&mut Node> {
        self.nodes
            .get_mut(arena.index as usize)
            .filter(|node| node.live && node.generation == arena.generation)
    }

    // A node whose generation reached the maximum is retired, so no stale id can name it again.
    fn vacant(&self) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| !node.live && node.generation != u32::MAX)
    }

    fn publish(&mut self, index: usize, reservation: Reservation) -> ArenaId {
        let node = &mut self.nodes[index];
        node.live = true;
        node.dying = false;
        node.earmarked = 0;
        node.tokens = 0;
        node.cursor = 0;
        node.reservation = reservation;
        ArenaId {
            index: index as u32,
            generation: node.generation,
        }
    }
}

pub const fn capacity() -> u32 {
    CAPACITY as u32
}

pub fn colours() -> u32 {
    super::colours().count() as u32
}

pub fn colour_mask(bits: u64) -> Option<ColourMask> {
    ColourMask::new(super::colours(), bits)
}

pub fn full_mask() -> ColourMask {
    ColourMask::full(super::colours())
}

// Leaves `ceil(slack / margin_divisor)` frames of every colour to the kernel's own allocations.
pub fn reserve_root(context: &mut ThreadContext, margin_divisor: u32) -> Result<ArenaId, Refused> {
    let _ = context;
    if margin_divisor == 0 {
        return Err(Refused::NoMemory);
    }
    let colours = super::colours();
    let mut guard = FRAMES.lock_leaf();
    let Frames { space, pool, nodes } = &mut *guard;
    if nodes.root {
        return Err(Refused::Twice);
    }
    let index = nodes.vacant().ok_or(Refused::NoNode)?;
    let mut amounts = ColourCounts::ZERO;
    for colour in 0..colours.count() {
        let slack = space
            .free_in(colours, colour)
            .saturating_sub(pool.unused(colour));
        let share = slack - slack.div_ceil(margin_divisor as usize);
        assert!(
            amounts.set(colour, share),
            "a colour below the count is in range"
        );
    }
    let mut reservation = Reservation::new(ColourMask::full(colours));
    if !space.reserve(colours, pool, &mut reservation, &amounts) {
        return Err(Refused::NoMemory);
    }
    nodes.root = true;
    Ok(nodes.publish(index, reservation))
}

pub fn donate(
    context: &mut ThreadContext,
    parent: ArenaId,
    mask: ColourMask,
    per_colour: u32,
) -> Result<ArenaId, Refused> {
    let _ = context;
    let colours = super::colours();
    let mut guard = FRAMES.lock_leaf();
    let nodes = &mut guard.nodes;
    let index = nodes.vacant().ok_or(Refused::NoNode)?;
    let parent = nodes
        .live(parent)
        .filter(|node| !node.dying)
        .ok_or(Refused::Stale)?;
    if !mask.is_within(parent.reservation.mask()) {
        return Err(Refused::Mask);
    }
    let mut amounts = ColourCounts::ZERO;
    let mut donated = 0usize;
    for colour in (0..colours.count()).filter(|&colour| mask.contains(colour)) {
        assert!(
            amounts.set(colour, per_colour as usize),
            "a colour below the count is in range"
        );
        donated += per_colour as usize;
    }
    // Earmarked frames are promised out of the parent's unused share, so a donation leaves them.
    let kept = parent.reservation.unused_total().checked_sub(donated);
    if kept.is_none_or(|kept| kept < parent.earmarked) {
        return Err(Refused::NoMemory);
    }
    let mut child = Reservation::new(mask);
    if !parent.reservation.transfer(&mut child, &amounts) {
        return Err(Refused::NoMemory);
    }
    Ok(nodes.publish(index, child))
}

pub fn earmark(
    context: &mut ThreadContext,
    arena: ArenaId,
    frames: u32,
) -> Result<Earmark, Refused> {
    let _ = context;
    let mut guard = FRAMES.lock_leaf();
    let node = guard
        .nodes
        .live(arena)
        .filter(|node| !node.dying)
        .ok_or(Refused::Stale)?;
    let wanted = frames as usize;
    let available = node
        .reservation
        .unused_total()
        .saturating_sub(node.earmarked);
    if wanted > available {
        return Err(Refused::NoMemory);
    }
    node.earmarked += wanted;
    Ok(Earmark {
        arena,
        left: wanted,
    })
}

// Frames promised to one operation; whatever it does not take returns when it drops.
#[derive(Debug)]
pub struct Earmark {
    arena: ArenaId,
    left: usize,
}

impl Earmark {
    pub fn take(&mut self, kind: FrameKind) -> Option<ArenaFrame> {
        if self.left == 0 {
            return None;
        }
        let colours = super::colours();
        let frame = {
            let mut guard = FRAMES.lock_leaf();
            let Frames { space, pool, nodes } = &mut *guard;
            let Some(node) = nodes.live(self.arena) else {
                panic!("an earmark outlived its arena");
            };
            let Some(frame) = space.allocate_reserved(colours, pool, &mut node.reservation) else {
                panic!("an earmarked frame was not allocatable");
            };
            table::claim(frame, Owner::Arena(self.arena.record(), kind));
            node.earmarked -= 1;
            node.tokens += 1;
            frame
        };
        self.left -= 1;
        // SAFETY: the frame was just taken, so this is its only handle, and it is writable.
        unsafe { zero_frame(frame) };
        Some(ArenaFrame {
            frame,
            arena: self.arena,
            kind,
        })
    }
}

impl Drop for Earmark {
    fn drop(&mut self) {
        if self.left == 0 {
            return;
        }
        if let Some(node) = FRAMES.lock_leaf().nodes.live(self.arena) {
            node.earmarked -= self.left;
        }
    }
}

// Writable, charged to its arena, and reached only through the direct map.
#[derive(Debug)]
pub struct ArenaFrame {
    frame: usize,
    arena: ArenaId,
    kind: FrameKind,
}

impl ArenaFrame {
    pub const fn address(&self) -> super::PhysicalAddress {
        frame_address(self.frame)
    }

    pub const fn arena(&self) -> ArenaId {
        self.arena
    }

    pub const fn kind(&self) -> FrameKind {
        self.kind
    }

    fn span(&self) -> DirectSpan<'_> {
        // SAFETY: this handle keeps the frame charged to its arena, its leaf is writable while it
        // is an `ArenaFrame`, and its bytes back no Rust value.
        let span = unsafe { DirectSpan::from_proven_run(self.address(), FRAME_BYTES as u64) };
        span.unwrap_or_else(|| panic!("an arena frame lost its direct-map address"))
    }

    #[must_use = "a refused frame write changed no bytes"]
    pub fn write_at(&self, at: usize, bytes: &[u8]) -> bool {
        self.span().write_at(at, bytes)
    }

    #[must_use = "a refused frame read copied no bytes"]
    pub fn read_at(&self, at: usize, into: &mut [u8]) -> bool {
        self.span().read_at(at, into)
    }

    pub fn surrender(self) {
        let frame = ManuallyDrop::new(self);
        surrender(frame.arena);
    }

    // Consuming the frame proves no writable mapping of it remains, since a mapping consumes it.
    pub fn seal(self, flush: &mut Flush) -> SealedFrame {
        assert!(
            self.kind != FrameKind::Table,
            "a page-table frame is never sealed"
        );
        let frame = ManuallyDrop::new(self);
        {
            let _guard = FRAMES.lock_leaf();
            table::seal(frame.frame);
            flush.merge(set_direct_access(frame.address(), DirectAccess::ReadOnly));
        }
        SealedFrame {
            frame: frame.frame,
            arena: frame.arena,
        }
    }
}

impl Drop for ArenaFrame {
    fn drop(&mut self) {
        // SAFETY: this is the frame's only handle and its leaf is writable.
        unsafe { zero_frame(self.frame) };
        settle(self.frame, self.arena, Access::Writable);
    }
}

// Read-only everywhere, the direct map included; it becomes writable only by being released.
#[derive(Debug)]
pub struct SealedFrame {
    frame: usize,
    arena: ArenaId,
}

impl SealedFrame {
    pub const fn address(&self) -> super::PhysicalAddress {
        frame_address(self.frame)
    }

    pub const fn arena(&self) -> ArenaId {
        self.arena
    }

    #[must_use = "a refused frame read copied no bytes"]
    pub fn read_at(&self, at: usize, into: &mut [u8]) -> bool {
        // SAFETY: this handle keeps the frame charged and sealed, its bytes back no Rust value, and
        // the span is only read.
        let span = unsafe { DirectSpan::from_proven_run(self.address(), FRAME_BYTES as u64) };
        span.is_some_and(|span| span.read_at(at, into))
    }

    pub fn surrender(self) {
        let frame = ManuallyDrop::new(self);
        surrender(frame.arena);
    }

    // The caller must already have removed every mapping of the frame and waited for that flush.
    pub fn release(self, flush: &mut Flush) {
        let frame = ManuallyDrop::new(self);
        release_sealed(frame.frame, frame.arena, flush);
    }
}

impl Drop for SealedFrame {
    fn drop(&mut self) {
        let mut flush = Flush::none();
        release_sealed(self.frame, self.arena, &mut flush);
        assert!(
            flush.is_complete(),
            "a dropped sealed frame awaits a flush its drop cannot wait for"
        );
    }
}

// The record goes free before the leaf turns writable, so no sealed frame is ever writable.
fn release_sealed(frame: usize, arena: ArenaId, flush: &mut Flush) {
    {
        let _guard = FRAMES.lock_leaf();
        let previous = table::read(frame);
        assert!(
            matches!(previous.owner(), Owner::Arena(index, _) if index == arena.record())
                && previous.access() == Access::Sealed,
            "frame {frame} was released as sealed while it is {previous:?}"
        );
        table::release(frame);
        flush.merge(set_direct_access(
            frame_address(frame),
            DirectAccess::Writable,
        ));
    }
    // SAFETY: the frame is still charged and taken, this is its only handle, and it is writable.
    unsafe { zero_frame(frame) };
    settle(frame, arena, Access::Sealed);
}

// Returns a zeroed frame to the bitmap and to its arena's reservation.
fn settle(frame: usize, arena: ArenaId, was: Access) {
    let colours = super::colours();
    let mut guard = FRAMES.lock_leaf();
    let Frames { space, pool, nodes } = &mut *guard;
    let Some(node) = nodes.live(arena) else {
        panic!("an arena frame outlived its arena");
    };
    if was == Access::Writable {
        let previous = table::read(frame);
        assert!(
            matches!(previous.owner(), Owner::Arena(index, _) if index == arena.record())
                && previous.access() == Access::Writable,
            "frame {frame} was released by arena {} while it is {previous:?}",
            arena.index
        );
        table::release(frame);
    }
    let freed = space.free_reserved(frame, colours, pool, &mut node.reservation);
    assert!(
        freed.is_ok(),
        "arena frame {frame} was not charged: {freed:?}"
    );
    node.tokens -= 1;
}

// Ends a token in constant time; its frame stays charged until `dissolve` zeroes and frees it.
fn surrender(arena: ArenaId) {
    let mut guard = FRAMES.lock_leaf();
    let Some(node) = guard.nodes.live(arena) else {
        panic!("a frame token outlived its arena");
    };
    node.tokens -= 1;
}

pub fn release(frame: ArenaFrame) {
    drop(frame);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dissolved {
    Progress,
    Done,
}

// Refuses until every token is released or surrendered, so no safe handle reaches what it frees;
// examines at most `budget` frame records per call, so teardown stays a bounded step.
pub fn dissolve(arena: ArenaId, heir: ArenaId, budget: u32) -> Result<Dissolved, Refused> {
    let colours = super::colours();
    let mut guard = FRAMES.lock_leaf();
    let Frames { space, pool, nodes } = &mut *guard;
    if arena == heir || nodes.live(heir).is_none() {
        return Err(Refused::Stale);
    }
    let limit = space.limit();
    let node = nodes.live(arena).ok_or(Refused::Stale)?;
    if node.earmarked != 0 || node.tokens != 0 {
        return Err(Refused::Busy);
    }
    node.dying = true;
    let end = node.cursor.saturating_add(budget as usize).min(limit);
    while node.cursor < end {
        let frame = node.cursor;
        node.cursor += 1;
        let record = table::read(frame);
        if !matches!(record.owner(), Owner::Arena(index, _) if index == arena.record()) {
            continue;
        }
        table::release(frame);
        if record.access() == Access::Sealed {
            let flush = set_direct_access(frame_address(frame), DirectAccess::Writable);
            assert!(
                flush.is_complete(),
                "a dissolved sealed frame awaits a flush"
            );
        }
        // SAFETY: the record named this dying arena, whose handles are never used again, and
        // its leaf is writable.
        unsafe { zero_frame(frame) };
        let freed = space.free_reserved(frame, colours, pool, &mut node.reservation);
        assert!(
            freed.is_ok(),
            "arena frame {frame} was not charged: {freed:?}"
        );
    }
    if node.cursor < limit {
        return Ok(Dissolved::Progress);
    }
    let mut reservation = core::mem::replace(&mut node.reservation, Reservation::EMPTY);
    let Some(heir) = nodes.live(heir) else {
        return Err(Refused::Stale);
    };
    if !reservation.return_to(&mut heir.reservation) {
        let Some(node) = nodes.live(arena) else {
            panic!("a dissolving arena vanished under the frame lock");
        };
        node.reservation = reservation;
        return Err(Refused::Mask);
    }
    let node = &mut nodes.nodes[arena.index as usize];
    node.live = false;
    node.dying = false;
    node.generation = node.generation.saturating_add(1);
    Ok(Dissolved::Done)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Usage {
    reserved: ColourCounts,
    used: ColourCounts,
}

impl Usage {
    pub const fn reserved_in(&self, colour: usize) -> usize {
        self.reserved.get(colour)
    }

    pub const fn used_in(&self, colour: usize) -> usize {
        self.used.get(colour)
    }

    pub fn reserved(&self) -> usize {
        (0..MAX_COLOURS)
            .map(|colour| self.reserved.get(colour))
            .sum()
    }

    pub fn used(&self) -> usize {
        (0..MAX_COLOURS).map(|colour| self.used.get(colour)).sum()
    }
}

pub fn usage(arena: ArenaId) -> Option<Usage> {
    let mut guard = FRAMES.lock_leaf();
    let node = guard.nodes.live(arena)?;
    let mut usage = Usage {
        reserved: ColourCounts::ZERO,
        used: ColourCounts::ZERO,
    };
    for colour in 0..MAX_COLOURS {
        let reserved = usage
            .reserved
            .set(colour, node.reservation.reserved(colour));
        let used = usage.used.set(colour, node.reservation.used(colour));
        assert!(reserved && used, "a colour below the maximum is in range");
    }
    Some(usage)
}
