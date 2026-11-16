#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
pub const DESCRIPTORS: u8 = 16;
#[verus_verify]
pub const BYTES: u16 = 2304;
#[verus_verify]
pub const HANDLES: u16 = 32;
#[verus_verify]
pub const MESSAGE_BYTES: u16 = 2048;
#[verus_verify]
pub const MESSAGE_HANDLES: u16 = 16;

#[verus_verify]
const SLOTS: usize = DESCRIPTORS as usize;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub at: u16,
    pub len: u16,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descriptor {
    pub(crate) bytes_at: u16,
    pub(crate) bytes: u16,
    pub(crate) handles_at: u16,
    pub(crate) handles: u16,
}

verus! {

exec const EMPTY: Descriptor = Descriptor { bytes_at: 0, bytes: 0, handles_at: 0, handles: 0 };

} // verus!

#[verus_verify]
impl Descriptor {
    #[verus_spec(r =>
        ensures
            r == self.spec_bytes(),
            r <= MESSAGE_BYTES,
    )]
    pub fn bytes(self) -> u16 {
        proof! {
            use_type_invariant(&self);
        }
        self.bytes
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_handles(),
            r <= MESSAGE_HANDLES,
    )]
    pub fn handles(self) -> u16 {
        proof! {
            use_type_invariant(&self);
        }
        self.handles
    }

    // The second span is empty unless the message wraps past the end of the ring.
    #[verus_spec(r =>
        ensures
            crate::proof::spans_cover(r, self.spec_bytes_at(), self.spec_bytes(), BYTES),
    )]
    pub fn byte_spans(self) -> [Span; 2] {
        proof! {
            use_type_invariant(&self);
        }
        spans(self.bytes_at, self.bytes, BYTES)
    }

    #[verus_spec(r =>
        ensures
            crate::proof::spans_cover(r, self.spec_handles_at(), self.spec_handles(), HANDLES),
    )]
    pub fn handle_spans(self) -> [Span; 2] {
        proof! {
            use_type_invariant(&self);
        }
        spans(self.handles_at, self.handles, HANDLES)
    }
}

#[verus_spec(r =>
    requires
        at < capacity,
        len <= capacity,
    ensures
        crate::proof::spans_cover(r, at, len, capacity),
)]
fn spans(at: u16, len: u16, capacity: u16) -> [Span; 2] {
    let room = capacity - at;
    if len <= room {
        [Span { at, len }, Span { at: 0, len: 0 }]
    } else {
        [
            Span { at, len: room },
            Span {
                at: 0,
                len: len - room,
            },
        ]
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub index: u8,
    pub descriptor: Descriptor,
}

// Three rings in FIFO order; a message's bytes and handles are freed exactly when it is popped.
#[verus_verify]
#[derive(PartialEq, Eq, Debug)]
pub struct Inbox {
    pub(crate) descriptors: [Descriptor; SLOTS],
    pub(crate) head: u8,
    pub(crate) count: u8,
    pub(crate) bytes_head: u16,
    pub(crate) bytes_used: u16,
    pub(crate) handles_head: u16,
    pub(crate) handles_used: u16,
}

verus! {

impl Inbox {
    pub exec const EMPTY: Self = Self {
        descriptors: [EMPTY; SLOTS],
        head: 0,
        count: 0,
        bytes_head: 0,
        bytes_used: 0,
        handles_head: 0,
        handles_used: 0,
    };
}

} // verus!

// Written out because a derived clone has no specification to carry the type invariant.
#[verus_verify]
impl Clone for Inbox {
    fn clone(&self) -> Self {
        proof! {
            use_type_invariant(self);
        }
        Self {
            descriptors: self.descriptors,
            head: self.head,
            count: self.count,
            bytes_head: self.bytes_head,
            bytes_used: self.bytes_used,
            handles_head: self.handles_head,
            handles_used: self.handles_used,
        }
    }
}

#[verus_verify]
impl Inbox {
    #[verus_spec(r =>
        ensures
            r == self.spec_messages().len(),
            r <= DESCRIPTORS,
    )]
    pub fn count(&self) -> u8 {
        proof! {
            use_type_invariant(self);
        }
        self.count
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(()) => crate::proof::inbox_admits(*self, bytes, handles),
                Err(InboxRefusal::TooLarge) => bytes > MESSAGE_BYTES || handles > MESSAGE_HANDLES,
                Err(InboxRefusal::Full) => bytes <= MESSAGE_BYTES && handles <= MESSAGE_HANDLES
                    && !crate::proof::inbox_admits(*self, bytes, handles),
            },
    )]
    pub fn admits(&self, bytes: u16, handles: u16) -> Result<(), InboxRefusal> {
        proof! {
            use_type_invariant(self);
        }
        if bytes > MESSAGE_BYTES || handles > MESSAGE_HANDLES {
            return Err(InboxRefusal::TooLarge);
        }
        if self.count >= DESCRIPTORS
            || bytes > BYTES - self.bytes_used
            || handles > HANDLES - self.handles_used
        {
            return Err(InboxRefusal::Full);
        }
        Ok(())
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(entry) => crate::proof::inbox_admits(*old(self), bytes, handles)
                    && entry.descriptor.spec_bytes() == bytes
                    && entry.descriptor.spec_handles() == handles
                    && entry.index == crate::proof::slot_of(
                        *old(self),
                        old(self).spec_messages().len() as int,
                    )
                    && final(self).spec_messages()
                        == old(self).spec_messages().push(entry.descriptor),
                Err(_) => !crate::proof::inbox_admits(*old(self), bytes, handles)
                    && *final(self) == *old(self),
            },
    )]
    pub fn push(&mut self, bytes: u16, handles: u16) -> Result<Entry, InboxRefusal> {
        self.admits(bytes, handles)?;
        proof! {
            use_type_invariant(&*self);
        }
        let index = (self.head + self.count) % DESCRIPTORS;
        let descriptor = Descriptor {
            bytes_at: (self.bytes_head + self.bytes_used) % BYTES,
            bytes,
            handles_at: (self.handles_head + self.handles_used) % HANDLES,
            handles,
        };
        let mut descriptors = self.descriptors;
        descriptors[usize::from(index)] = descriptor;
        proof! {
            let before = self.spec_messages();
            let after = Self {
                descriptors,
                head: self.head,
                count: (self.count + 1) as u8,
                bytes_head: self.bytes_head,
                bytes_used: (self.bytes_used + bytes) as u16,
                handles_head: self.handles_head,
                handles_used: (self.handles_used + handles) as u16,
            };
            assert(after.spec_messages() =~= before.push(descriptor));
            crate::proof::lemma_used_after_push(before, descriptor);
            assert forall|k: int| 0 <= k <= before.len() implies crate::proof::tiled(
                after,
                k,
            ) by {
                if k < before.len() {
                    assert(crate::proof::tiled(*self, k));
                }
            }
        }
        *self = Self {
            descriptors,
            head: self.head,
            count: self.count + 1,
            bytes_head: self.bytes_head,
            bytes_used: self.bytes_used + bytes,
            handles_head: self.handles_head,
            handles_used: self.handles_used + handles,
        };
        Ok(Entry { index, descriptor })
    }

    #[verus_spec(r =>
        ensures
            r == self.get_spec(0),
    )]
    pub fn head(&self) -> Option<Entry> {
        self.get(0)
    }

    #[verus_spec(r =>
        ensures
            r == self.get_spec(position as int),
            r is Some ==> crate::proof::handles_in_ring(*self, position as int),
    )]
    pub fn get(&self, position: u8) -> Option<Entry> {
        proof! {
            use_type_invariant(self);
        }
        if position >= self.count {
            return None;
        }
        let index = (self.head + position) % DESCRIPTORS;
        proof! {
            crate::proof::lemma_handles_before_bounded(self.spec_messages(), position as int);
        }
        Some(Entry {
            index,
            descriptor: self.descriptors[usize::from(index)],
        })
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(entry) => old(self).spec_messages().len() > 0
                    && r == old(self).get_spec(0)
                    && final(self).spec_messages() == old(self).spec_messages().drop_first(),
                None => old(self).spec_messages().len() == 0 && *final(self) == *old(self),
            },
    )]
    pub fn pop(&mut self) -> Option<Entry> {
        let entry = self.head()?;
        proof! {
            use_type_invariant(&*self);
        }
        let descriptor = entry.descriptor;
        let mut descriptors = self.descriptors;
        descriptors[usize::from(entry.index)] = EMPTY;
        proof! {
            let before = self.spec_messages();
            crate::proof::lemma_used_after_pop(before);
            let after = Self {
                descriptors,
                head: ((self.head + 1) % 16) as u8,
                count: (self.count - 1) as u8,
                bytes_head: ((self.bytes_head + descriptor.bytes) % 2304) as u16,
                bytes_used: (self.bytes_used - descriptor.bytes) as u16,
                handles_head: ((self.handles_head + descriptor.handles) % 32) as u16,
                handles_used: (self.handles_used - descriptor.handles) as u16,
            };
            assert forall|k: int| 0 <= k < before.len() - 1 implies after.spec_messages()[k]
                == before.drop_first()[k] by {
                crate::proof::lemma_mod_shift(self.head + 1, k, 16);
            }
            assert(after.spec_messages() =~= before.drop_first());
            assert forall|k: int| 0 <= k < before.len() - 1 implies crate::proof::tiled(
                after,
                k,
            ) by {
                assert(crate::proof::tiled(*self, k + 1));
                crate::proof::lemma_mod_shift(
                    self.bytes_head + descriptor.bytes,
                    crate::proof::bytes_before(before.drop_first(), k),
                    2304,
                );
                crate::proof::lemma_mod_shift(
                    self.handles_head + descriptor.handles,
                    crate::proof::handles_before(before.drop_first(), k),
                    32,
                );
            }
        }
        *self = Self {
            descriptors,
            head: (self.head + 1) % DESCRIPTORS,
            count: self.count - 1,
            bytes_head: (self.bytes_head + descriptor.bytes) % BYTES,
            bytes_used: self.bytes_used - descriptor.bytes,
            handles_head: (self.handles_head + descriptor.handles) % HANDLES,
            handles_used: self.handles_used - descriptor.handles,
        };
        Some(entry)
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InboxRefusal {
    TooLarge,
    Full,
}
