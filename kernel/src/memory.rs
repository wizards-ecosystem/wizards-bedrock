use core::sync::atomic::{AtomicU64, Ordering};

mod quota;

use memory_object::{ObjectState, Seal, StateRefusal, StateWord};
use native_abi::Rights;
use world_tree::NodeRef;
use wost::arch::current::paging::{Flush, FrameRef};
use wost::mm::Sealing;
use wost::mm::pool::{ArenaFrame, Earmark, FrameKind, SealedFrame};
use wost::sync::{Leaf, SpinLock};
use x86_paging::PAGE_SIZE;

pub(crate) use quota::CodeQuota;

use crate::arena::Pages;
use crate::rank;
use crate::thread::ThreadContext;
use crate::world::registry;

const PER_BLOCK: u32 = 64;

const PAGE_BYTES: usize = PAGE_SIZE as usize;

#[derive(Debug)]
enum Frame {
    Writable(ArenaFrame),
    Sealing(Sealing<SealedFrame>),
    Sealed(SealedFrame),
}

#[derive(Debug)]
struct Block {
    frames: [Option<Frame>; PER_BLOCK as usize],
}

impl Block {
    const fn empty() -> Self {
        Self {
            frames: [const { None }; PER_BLOCK as usize],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    // One handle, never duplicated, so a seal can consume it.
    Sealable,
    Shareable,
}

impl Kind {
    pub(crate) const fn rights(self) -> Rights {
        let held = Rights::TRANSFER
            .union(Rights::READ)
            .union(Rights::WRITE)
            .union(Rights::MAP);
        match self {
            Self::Sealable => held,
            Self::Shareable => held.union(Rights::DUPLICATE),
        }
    }
}

pub(crate) const SEALED_RIGHTS: Rights = Rights::DUPLICATE
    .union(Rights::TRANSFER)
    .union(Rights::READ)
    .union(Rights::MAP);

// The quota an execute seal is charged to, and the world that keeps it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Payer<'quota> {
    pub(crate) world: NodeRef,
    pub(crate) quota: &'quota CodeQuota,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SealRefusal {
    State(StateRefusal),
    NoQuota,
}

// Pages of arena data frames, writable until a seal turns every one sealed at once.
#[derive(Debug)]
pub(crate) struct MemoryObject {
    state: AtomicU64,
    kind: Kind,
    pages: u32,
    frames: SpinLock<rank::Object, Pages<Block>>,
    charged: SpinLock<Leaf, Option<NodeRef>>,
}

impl MemoryObject {
    // The block pages and one data frame per page; `None` when no arena could hold them.
    pub(crate) fn frames_for(pages: u32) -> Option<u32> {
        let blocks = pages.div_ceil(PER_BLOCK);
        if blocks > object_pool::PAGES {
            return None;
        }
        Pages::<Block>::frames_for(blocks).checked_add(pages)
    }

    pub(crate) fn new(earmark: &mut Earmark, kind: Kind, pages: u32) -> Self {
        let mut blocks = Pages::build(earmark, pages.div_ceil(PER_BLOCK), Block::empty);
        for page in 0..pages {
            let Some(frame) = earmark.take(FrameKind::ArenaData) else {
                panic!("a memory object outran the earmark its pages were counted for");
            };
            *slot_mut(&mut blocks, page) = Some(Frame::Writable(frame));
        }
        Self {
            state: AtomicU64::new(StateWord::WRITABLE.bits()),
            kind,
            pages,
            frames: SpinLock::new(blocks),
            charged: SpinLock::new(None),
        }
    }

    pub(crate) const fn kind(&self) -> Kind {
        self.kind
    }

    pub(crate) fn state(&self) -> ObjectState {
        match StateWord::from_bits(self.state.load(Ordering::Acquire)) {
            Some(word) => word.state(),
            None => panic!("a memory object's state word is not a state"),
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        u64::from(self.pages) * PAGE_SIZE
    }

    fn transition(
        &self,
        next: impl Fn(StateWord) -> Result<StateWord, StateRefusal>,
    ) -> Result<(), StateRefusal> {
        let mut seen = self.state.load(Ordering::Acquire);
        loop {
            let Some(word) = StateWord::from_bits(seen) else {
                panic!("a memory object's state word {seen:#x} is not a state");
            };
            let wanted = next(word)?.bits();
            match self.state.compare_exchange_weak(
                seen,
                wanted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(current) => seen = current,
            }
        }
    }

    pub(crate) fn acquire_writer(&self) -> Result<(), StateRefusal> {
        self.transition(StateWord::acquire_writer)
    }

    pub(crate) fn release_writer(&self) {
        if let Err(refusal) = self.transition(StateWord::release_writer) {
            panic!("a memory object's writer was released twice: {refusal}");
        }
    }

    // `false` when the range runs past the object.
    pub(crate) fn read(&self, context: &mut ThreadContext, at: u64, into: &mut [u8]) -> bool {
        if !self.covers(at, into.len()) {
            return false;
        }
        let mut blocks = self.frames.lock(context);
        let mut done = 0;
        while done < into.len() {
            let (page, offset, len) = span(at, done, into.len());
            let chunk = &mut into[done..done + len];
            let read = match slot_mut(&mut blocks, page) {
                Some(Frame::Writable(frame)) => frame.read_at(offset, chunk),
                Some(Frame::Sealing(frame)) => frame.sealed().read_at(offset, chunk),
                Some(Frame::Sealed(frame)) => frame.read_at(offset, chunk),
                None => false,
            };
            assert!(read, "a covered page of a memory object refused a read");
            done += len;
        }
        true
    }

    // The caller holds a writer, so no seal can turn a frame sealed under it.
    pub(crate) fn write(&self, context: &mut ThreadContext, at: u64, bytes: &[u8]) -> bool {
        if !self.covers(at, bytes.len()) {
            return false;
        }
        let mut blocks = self.frames.lock(context);
        let mut done = 0;
        while done < bytes.len() {
            let (page, offset, len) = span(at, done, bytes.len());
            let Some(Frame::Writable(frame)) = slot_mut(&mut blocks, page) else {
                panic!("a memory object with a writer held a page that is not writable");
            };
            assert!(
                frame.write_at(offset, &bytes[done..done + len]),
                "a covered page of a memory object refused a write"
            );
            done += len;
        }
        true
    }

    fn covers(&self, at: u64, len: usize) -> bool {
        at.checked_add(len as u64)
            .is_some_and(|end| end <= self.bytes())
    }

    // Only from writable with no writer; an execute seal is charged before the state moves and
    // credited back on a refusal. Every frame is sealed and flushed before this returns.
    pub(crate) fn seal(
        &self,
        context: &mut ThreadContext,
        seal: Seal,
        payer: Option<Payer<'_>>,
    ) -> Result<(), SealRefusal> {
        let payer = match (seal, payer) {
            (Seal::Execute, None) => return Err(SealRefusal::NoQuota),
            (Seal::Execute, Some(payer)) => Some(payer),
            (Seal::Read, _) => None,
        };
        if let Some(payer) = payer
            && payer.quota.debit(self.pages).is_err()
        {
            return Err(SealRefusal::NoQuota);
        }
        if let Err(refusal) = self.transition(|word| word.seal(seal)) {
            if let Some(payer) = payer {
                payer.quota.credit(self.pages);
            }
            return Err(SealRefusal::State(refusal));
        }
        *self.charged.lock_leaf() = payer.map(|payer| payer.world);
        let mut flush = Flush::none();
        {
            let mut blocks = self.frames.lock(context);
            for page in 0..self.pages {
                let slot = slot_mut(&mut blocks, page);
                let Some(Frame::Writable(frame)) = slot.take() else {
                    panic!("a memory object sealed a page that was not writable");
                };
                *slot = Some(Frame::Sealing(frame.seal(&mut flush)));
            }
        }
        flush.wait(context);
        let mut blocks = self.frames.lock(context);
        for page in 0..self.pages {
            let slot = slot_mut(&mut blocks, page);
            let Some(Frame::Sealing(sealing)) = slot.take() else {
                panic!("a memory object's sealing page changed under its seal");
            };
            let Ok(frame) = sealing.finish() else {
                panic!("a memory object's page awaited its flush and is still not sealed");
            };
            *slot = Some(Frame::Sealed(frame));
        }
        Ok(())
    }

    // Each of `pages` frames from page `first`, under the object's lock, with its index in the run.
    pub(crate) fn frames<E>(
        &self,
        context: &mut ThreadContext,
        first: u32,
        pages: u64,
        mut visit: impl FnMut(u64, FrameRef<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut blocks = self.frames.lock(context);
        for index in 0..pages {
            let Some(page) = u32::try_from(index)
                .ok()
                .and_then(|index| first.checked_add(index))
                .filter(|page| *page < self.pages)
            else {
                panic!("a mapping record ran past its object");
            };
            let frame = match slot_mut(&mut blocks, page) {
                Some(Frame::Writable(frame)) => FrameRef::Writable(frame),
                Some(Frame::Sealed(frame)) => FrameRef::Sealed(frame),
                Some(Frame::Sealing(frame)) => FrameRef::Sealed(frame.sealed()),
                None => panic!("a memory object lost page {page}"),
            };
            visit(index, frame)?;
        }
        Ok(())
    }
}

// The frames go back to the arena here; a dropped object page would forget them.
impl Drop for MemoryObject {
    fn drop(&mut self) {
        let blocks = self.frames.get_mut();
        for page in 0..self.pages {
            drop(slot_mut(blocks, page).take());
        }
        if let Some(world) = self.charged.get_mut().take()
            && let Some(world) = registry::hold(world)
        {
            world.code.credit(self.pages);
        }
    }
}

fn slot_mut(blocks: &mut Pages<Block>, page: u32) -> &mut Option<Frame> {
    let Some(slot) = blocks
        .get_mut(page / PER_BLOCK)
        .and_then(|block| block.frames.get_mut((page % PER_BLOCK) as usize))
    else {
        panic!("a memory object lost the block of page {page}");
    };
    slot
}

// The page, offset in it and length of the next run of a copy that has done `done` bytes.
fn span(at: u64, done: usize, total: usize) -> (u32, usize, usize) {
    let position = at + done as u64;
    let offset = (position % PAGE_SIZE) as usize;
    let len = (PAGE_BYTES - offset).min(total - done);
    ((position / PAGE_SIZE) as u32, offset, len)
}
