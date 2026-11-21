use object_pool::{Index, frames_needed};
use wost::mm::pool::{self as frames, ArenaId, Earmark, FrameKind, Refused};
use wost::mm::slab::{Hold, ObjectPage, Refusal, Released, Removal};
use wost::sync::{Leaf, SpinLock};

use super::Local;
use super::directory::{Directory, Frames, LEVELS};
use crate::thread::ThreadContext;

const NONE: u32 = u32::MAX;

// Taking or releasing a frame locks the frames, so it happens outside this leaf.
#[derive(Debug)]
pub(crate) struct Pool<T, const N: usize> {
    state: SpinLock<Leaf, State<T, N>>,
}

#[derive(Debug)]
struct State<T, const N: usize> {
    arena: Option<ArenaId>,
    directory: Directory<Page<T, N>>,
    available: u32,
    spare: u32,
    live: u32,
}

#[derive(Debug)]
struct Page<T, const N: usize> {
    page: ObjectPage<T, N>,
    previous: u32,
    next: u32,
    listed: bool,
}

impl<T, const N: usize> Pool<T, N> {
    pub(crate) const fn new() -> Self {
        Self {
            state: SpinLock::new(State::EMPTY),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.state.lock_leaf().live == 0
    }

    // Every page returns to the arena; the pool can then serve another arena.
    pub(crate) fn close(&self) {
        let closed = core::mem::replace(&mut *self.state.lock_leaf(), State::EMPTY);
        assert!(closed.live == 0, "a pool closed while objects lived in it");
        drop(closed);
    }

    // Enough for every object to open a page and its directory path; inserts return the rest.
    pub(crate) fn frames_for(objects: u32) -> Option<u32> {
        let per_page = u16::try_from(N).ok()?;
        frames_needed(objects, 0, per_page)?.checked_mul(1 + LEVELS)
    }

    pub(crate) fn earmark(
        context: &mut ThreadContext,
        arena: ArenaId,
        objects: u32,
    ) -> Result<Earmark, Refused> {
        let wanted = Self::frames_for(objects).ok_or(Refused::NoMemory)?;
        frames::earmark(context, arena, wanted)
    }

    pub(crate) fn insert(&self, earmark: &mut Earmark, make: impl FnOnce() -> T) -> Local<T> {
        let mut taken = Frames::none();
        loop {
            let mut state = self.state.lock_leaf();
            // A listed page takes no frame, so the earmark itself names whose object this is.
            let owner = *state.arena.get_or_insert(earmark.arena());
            assert!(
                owner == earmark.arena(),
                "a pool of one arena was handed an earmark of another"
            );
            let needed = state.frames_to_insert();
            if taken.count() >= needed {
                let local = state.insert(&mut taken, make);
                drop(state);
                drop(taken);
                return local;
            }
            drop(state);
            while taken.count() < needed {
                let Some(frame) = earmark.take(FrameKind::ArenaObject) else {
                    panic!("an insert outran the earmark its pool sized");
                };
                taken.push(frame);
            }
        }
    }

    pub(crate) fn hold(&self, local: Local<T>) -> Option<Hold<T>>
    where
        T: Sync,
    {
        let mut state = self.state.lock_leaf();
        let page = state.directory.get_mut(local.index().page())?;
        page.page.hold(local.index().slot(), local.generation())
    }
}

impl<T, const N: usize> Pool<T, N> {
    // Drops one strong reference and hands back the object once none remain.
    pub(crate) fn destroy(&self, local: Local<T>) -> Result<Option<T>, Removal> {
        match self.release(local) {
            Ok(Released::Held) => Ok(None),
            Ok(Released::NowZero) | Err(Refusal::Condemned) => self.remove(local).map(Some),
            Err(refusal) => Err(Removal::Refused(refusal)),
        }
    }

    pub(crate) fn acquire(&self, local: Local<T>) -> Result<(), Refusal> {
        let mut state = self.state.lock_leaf();
        let page = state
            .directory
            .get_mut(local.index().page())
            .ok_or(Refusal::Stale)?;
        page.page.acquire(local.index().slot(), local.generation())
    }

    fn release(&self, local: Local<T>) -> Result<Released, Refusal> {
        let mut state = self.state.lock_leaf();
        let page = state
            .directory
            .get_mut(local.index().page())
            .ok_or(Refusal::Stale)?;
        page.page.release(local.index().slot(), local.generation())
    }

    // Keeps one empty page; any other page that empties closes, and its index keeps a floor.
    fn remove(&self, local: Local<T>) -> Result<T, Removal> {
        let (value, closed) = {
            let mut guard = self.state.lock_leaf();
            let state = &mut *guard;
            let ordinal = local.index().page();
            let page = state
                .directory
                .get_mut(ordinal)
                .ok_or(Removal::Refused(Refusal::Stale))?;
            let value = page.page.remove(local.index().slot(), local.generation())?;
            state.live -= 1;
            let (live, free, listed, floor) = (
                page.page.live(),
                page.page.free(),
                page.listed,
                page.page.floor(),
            );
            let another_spare = state.spare != NONE && state.spare != ordinal;
            let closed = if live == 0 && (free == 0 || another_spare) {
                if listed {
                    state.unlist(ordinal);
                }
                state.directory.vacate(ordinal, floor)
            } else {
                if live == 0 {
                    state.spare = ordinal;
                }
                if !listed && free > 0 {
                    state.list(ordinal);
                }
                None
            };
            (value, closed)
        };
        drop(closed);
        Ok(value)
    }
}

impl<T, const N: usize> State<T, N> {
    const EMPTY: Self = Self {
        arena: None,
        directory: Directory::EMPTY,
        available: NONE,
        spare: NONE,
        live: 0,
    };

    fn frames_to_insert(&self) -> u32 {
        if self.available == NONE {
            1 + self.directory.nodes_to_open()
        } else {
            0
        }
    }

    fn insert(&mut self, taken: &mut Frames, make: impl FnOnce() -> T) -> Local<T> {
        if self.available == NONE {
            let frame = taken.pop();
            let ordinal = self.directory.open(taken, |floor| Page {
                page: ObjectPage::open(frame, floor),
                previous: NONE,
                next: NONE,
                listed: false,
            });
            self.list(ordinal);
        }
        let ordinal = self.available;
        let page = self.page_mut(ordinal);
        let Some(issued) = page.page.insert_with(make) else {
            panic!("a listed object page had no free slot");
        };
        let full = page.page.free() == 0;
        self.live += 1;
        if self.spare == ordinal {
            self.spare = NONE;
        }
        if full {
            self.unlist(ordinal);
        }
        let Some(index) = Index::new(ordinal, issued.slot) else {
            panic!("page ordinal {ordinal} lies outside the index range");
        };
        Local::new(index, issued.generation)
    }

    fn page_mut(&mut self, ordinal: u32) -> &mut Page<T, N> {
        self.directory.get_mut(ordinal).unwrap_or_else(|| {
            panic!("the pool's page list named ordinal {ordinal}, which is not a page")
        })
    }

    fn list(&mut self, ordinal: u32) {
        let head = self.available;
        let page = self.page_mut(ordinal);
        page.listed = true;
        page.previous = NONE;
        page.next = head;
        if head != NONE {
            self.page_mut(head).previous = ordinal;
        }
        self.available = ordinal;
    }

    fn unlist(&mut self, ordinal: u32) {
        let page = self.page_mut(ordinal);
        page.listed = false;
        let (previous, next) = (page.previous, page.next);
        if previous == NONE {
            self.available = next;
        } else {
            self.page_mut(previous).next = next;
        }
        if next != NONE {
            self.page_mut(next).previous = previous;
        }
    }
}
