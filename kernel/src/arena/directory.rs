use core::mem;

use object_pool::PAGES;
use wost::mm::pool::ArenaId;
use wost::mm::slab::{ObjectPage, PageFrame};

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

// Frames taken from an earmark before the pool lock, since taking one locks the frames.
#[derive(Debug)]
pub(super) struct Frames {
    frames: [Option<PageFrame>; 1 + LEVELS as usize],
    arena: Option<ArenaId>,
}

impl Frames {
    pub(super) const fn none() -> Self {
        Self {
            frames: [const { None }; 1 + LEVELS as usize],
            arena: None,
        }
    }

    pub(super) const fn arena(&self) -> Option<ArenaId> {
        self.arena
    }

    pub(super) fn count(&self) -> u32 {
        self.frames.iter().filter(|frame| frame.is_some()).count() as u32
    }

    pub(super) fn push(&mut self, frame: PageFrame) {
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

    pub(super) fn pop(&mut self) -> PageFrame {
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
    #[expect(
        dead_code,
        reason = "only object destruction closes pages, and it starts with the handle tables"
    )]
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

fn position(ordinal: u32, level: u32) -> usize {
    (ordinal >> (BITS * (level - 1))) as usize & (FANOUT - 1)
}

fn new_node<E>(frame: PageFrame) -> ObjectPage<Node<E>, 1> {
    let mut page = ObjectPage::open(frame, 0);
    let node = Node {
        entries: [const { Entry::Unborn }; FANOUT],
    };
    assert!(
        page.insert(node).is_ok(),
        "a fresh directory node page had no free slot"
    );
    page
}

fn node_mut<E>(page: &mut ObjectPage<Node<E>, 1>) -> &mut Node<E> {
    page.get_mut(0, 0)
        .unwrap_or_else(|| panic!("a directory node page lost its node"))
}
