use core::mem;

use object_pool::PAGES;
use wost::mm::pool::{ArenaFrame, ArenaId, Earmark, FrameKind};
use wost::mm::slab::{Blank, ObjectPage};

const BITS: u32 = 6;
const FANOUT: usize = 1 << BITS;
pub(super) const LEVELS: u32 = 4;
const NONE: u32 = u32::MAX;

#[derive(Debug)]
enum Entry<E> {
    Unborn,
    Vacant { floor: u32, next: u32 },
    Retired,
    Occupied(E),
    Node(ObjectPage<Node<E>, 1>),
}

#[derive(Debug)]
struct Node<E> {
    entries: [Entry<E>; FANOUT],
}

impl<E> Blank for Node<E> {
    const BLANK: Self = Self {
        entries: [const { Entry::Unborn }; FANOUT],
    };
}

// Frames taken from an earmark before the pool lock, since taking one locks the frames.
#[derive(Debug)]
pub(super) struct Frames {
    frames: [Option<ArenaFrame>; 1 + LEVELS as usize],
    arena: Option<ArenaId>,
}

impl Frames {
    pub(super) const fn none() -> Self {
        Self {
            frames: [const { None }; 1 + LEVELS as usize],
            arena: None,
        }
    }

    pub(super) fn count(&self) -> u32 {
        self.frames.iter().filter(|frame| frame.is_some()).count() as u32
    }

    pub(super) fn push(&mut self, frame: ArenaFrame) {
        let arena = *self.arena.get_or_insert(frame.arena());
        assert!(
            arena == frame.arena(),
            "one earmark handed out frames of two arenas"
        );
        let Some(empty) = self.frames.iter_mut().find(|frame| frame.is_none()) else {
            panic!("an insert took more frames than one page and its directory path");
        };
        *empty = Some(frame);
    }

    pub(super) fn pop(&mut self) -> ArenaFrame {
        let frame = self.frames.iter_mut().find_map(Option::take);
        frame.unwrap_or_else(|| panic!("an insert ran out of the frames it counted"))
    }
}

// Ordinals to entries through 64-way nodes in the arena; the depth grows only as ordinals do.
#[derive(Debug)]
pub(super) struct Directory<E> {
    root: Entry<E>,
    depth: u32,
    fresh: u32,
    free: u32,
}

const fn span(depth: u32) -> u64 {
    1 << (BITS * depth)
}

impl<E> Directory<E> {
    pub(super) const EMPTY: Self = Self {
        root: Entry::Unborn,
        depth: 0,
        fresh: 0,
        free: NONE,
    };

    // Node frames the next `open` needs: none for a vacant ordinal, else the missing path.
    pub(super) fn nodes_to_open(&self) -> u32 {
        if self.free != NONE {
            return 0;
        }
        let ordinal = u64::from(self.fresh);
        let mut depth = self.depth;
        while ordinal >= span(depth) {
            depth += 1;
        }
        let mut missing = 0;
        for level in 1..=depth {
            let first = ordinal & !(span(level) - 1);
            if level > self.depth || first == ordinal {
                missing += 1;
            }
        }
        missing
    }

    pub(super) fn open(&mut self, frames: &mut Frames, make: impl FnOnce(u32) -> E) -> u32 {
        if self.free != NONE {
            let ordinal = self.free;
            let Some(entry) = self.entry_mut(ordinal) else {
                panic!("the free list named ordinal {ordinal}, which has no entry");
            };
            let Entry::Vacant { floor, next } = *entry else {
                panic!("the free list named ordinal {ordinal}, which is not vacant");
            };
            *entry = Entry::Occupied(make(floor));
            self.free = next;
            return ordinal;
        }
        let ordinal = self.fresh;
        assert!(ordinal < PAGES, "a pool used every page ordinal");
        while u64::from(ordinal) >= span(self.depth) {
            let mut node = new_node(frames.pop());
            let old = mem::replace(&mut self.root, Entry::Unborn);
            if let Some(first) = node_mut(&mut node).entries.first_mut() {
                *first = old;
            }
            self.root = Entry::Node(node);
            self.depth += 1;
        }
        let mut entry = &mut self.root;
        for level in (1..=self.depth).rev() {
            if matches!(entry, Entry::Unborn) {
                *entry = Entry::Node(new_node(frames.pop()));
            }
            let Entry::Node(page) = entry else {
                panic!("a directory level above the leaves holds no node");
            };
            let Some(next) = node_mut(page).entries.get_mut(position(ordinal, level)) else {
                panic!("a position is below the fanout");
            };
            entry = next;
        }
        *entry = Entry::Occupied(make(0));
        self.fresh += 1;
        ordinal
    }

    pub(super) fn get_mut(&mut self, ordinal: u32) -> Option<&mut E> {
        match self.entry_mut(ordinal)? {
            Entry::Occupied(value) => Some(value),
            _ => None,
        }
    }

    // A `None` floor retires the ordinal, so no later page reuses its generations.
    pub(super) fn vacate(&mut self, ordinal: u32, floor: Option<u32>) -> Option<E> {
        let next = self.free;
        let entry = self.entry_mut(ordinal)?;
        if !matches!(entry, Entry::Occupied(_)) {
            return None;
        }
        let vacant = match floor {
            Some(floor) => Entry::Vacant { floor, next },
            None => Entry::Retired,
        };
        let Entry::Occupied(value) = mem::replace(entry, vacant) else {
            panic!("an occupied entry changed under the pool lock");
        };
        if floor.is_some() {
            self.free = ordinal;
        }
        Some(value)
    }

    fn entry_mut(&mut self, ordinal: u32) -> Option<&mut Entry<E>> {
        if u64::from(ordinal) >= span(self.depth) {
            return None;
        }
        let mut entry = &mut self.root;
        for level in (1..=self.depth).rev() {
            let Entry::Node(page) = entry else {
                return None;
            };
            entry = node_mut(page).entries.get_mut(position(ordinal, level))?;
        }
        Some(entry)
    }
}

// An object page forgets its objects, so a node hands back every entry before its own frame goes.
impl<E> Drop for Directory<E> {
    fn drop(&mut self) {
        release(mem::replace(&mut self.root, Entry::Unborn));
    }
}

fn release<E>(entry: Entry<E>) {
    if let Entry::Node(mut page) = entry {
        for entry in &mut node_mut(&mut page).entries {
            release(mem::replace(entry, Entry::Unborn));
        }
    }
}

fn position(ordinal: u32, level: u32) -> usize {
    (ordinal >> (BITS * (level - 1))) as usize & (FANOUT - 1)
}

fn new_node<E>(frame: ArenaFrame) -> ObjectPage<Node<E>, 1> {
    let mut page = ObjectPage::open(frame, 0);
    assert!(
        page.insert_blank().is_some(),
        "a fresh directory node page had no free slot"
    );
    page
}

fn node_mut<E>(page: &mut ObjectPage<Node<E>, 1>) -> &mut Node<E> {
    page.get_mut(0, 0)
        .unwrap_or_else(|| panic!("a directory node page lost its node"))
}

// A dense run of blocks, one object page each, built whole from an earmark and never shrunk.
#[derive(Debug)]
pub(crate) struct Pages<B> {
    directory: Directory<ObjectPage<B, 1>>,
}

impl<B: Blank> Pages<B> {
    pub(crate) fn frames_for(count: u32) -> u32 {
        let mut frames = count;
        let mut depth = 0;
        while u64::from(count) > span(depth) {
            depth += 1;
        }
        for level in 1..=depth {
            frames += u64::from(count).div_ceil(span(level)) as u32;
        }
        frames
    }

    pub(crate) fn build(earmark: &mut Earmark, count: u32) -> Self {
        let mut directory = Directory::EMPTY;
        for _ in 0..count {
            let mut frames = Frames::none();
            for _ in 0..=directory.nodes_to_open() {
                let Some(frame) = earmark.take(FrameKind::ArenaObject) else {
                    panic!("a block run outran the earmark its frames were counted for");
                };
                frames.push(frame);
            }
            let mut page = ObjectPage::open(frames.pop(), 0);
            assert!(
                page.insert_blank().is_some(),
                "a fresh block page had no free slot"
            );
            directory.open(&mut frames, |_| page);
        }
        Self { directory }
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut B> {
        self.directory.get_mut(index)?.get_mut(0, 0)
    }
}
