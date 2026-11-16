use core::fmt;

pub const DESCRIPTORS: u8 = 16;
pub const BYTES: u16 = 2304;
pub const HANDLES: u16 = 32;
pub const MESSAGE_BYTES: u16 = 2048;
pub const MESSAGE_HANDLES: u16 = 16;

const SLOTS: usize = DESCRIPTORS as usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub at: u16,
    pub len: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descriptor {
    bytes_at: u16,
    bytes: u16,
    handles_at: u16,
    handles: u16,
}

const EMPTY: Descriptor = Descriptor {
    bytes_at: 0,
    bytes: 0,
    handles_at: 0,
    handles: 0,
};

impl Descriptor {
    pub fn bytes(self) -> u16 {
        self.bytes
    }

    pub fn handles(self) -> u16 {
        self.handles
    }

    // The second span is empty unless the message wraps past the end of the ring.
    pub fn byte_spans(self) -> [Span; 2] {
        spans(self.bytes_at, self.bytes, BYTES)
    }

    pub fn handle_spans(self) -> [Span; 2] {
        spans(self.handles_at, self.handles, HANDLES)
    }
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub index: u8,
    pub descriptor: Descriptor,
}

// Three rings in FIFO order; a message's bytes and handles are freed exactly when it is popped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Inbox {
    descriptors: [Descriptor; SLOTS],
    head: u8,
    count: u8,
    bytes_head: u16,
    bytes_used: u16,
    handles_head: u16,
    handles_used: u16,
}

impl Inbox {
    pub const EMPTY: Self = Self {
        descriptors: [EMPTY; SLOTS],
        head: 0,
        count: 0,
        bytes_head: 0,
        bytes_used: 0,
        handles_head: 0,
        handles_used: 0,
    };

    pub fn count(&self) -> u8 {
        self.count
    }

    pub fn admits(&self, bytes: u16, handles: u16) -> Result<(), InboxRefusal> {
        if bytes > MESSAGE_BYTES || handles > MESSAGE_HANDLES {
            return Err(InboxRefusal::TooLarge);
        }
        if self.count >= DESCRIPTORS
            || self.bytes_used > BYTES
            || self.handles_used > HANDLES
            || bytes > BYTES - self.bytes_used
            || handles > HANDLES - self.handles_used
        {
            return Err(InboxRefusal::Full);
        }
        Ok(())
    }

    pub fn push(&mut self, bytes: u16, handles: u16) -> Result<Entry, InboxRefusal> {
        self.admits(bytes, handles)?;
        let index = (self.head + self.count) % DESCRIPTORS;
        let descriptor = Descriptor {
            bytes_at: (self.bytes_head + self.bytes_used) % BYTES,
            bytes,
            handles_at: (self.handles_head + self.handles_used) % HANDLES,
            handles,
        };
        self.descriptors[usize::from(index)] = descriptor;
        self.count += 1;
        self.bytes_used += bytes;
        self.handles_used += handles;
        Ok(Entry { index, descriptor })
    }

    pub fn head(&self) -> Option<Entry> {
        self.get(0)
    }

    pub fn get(&self, position: u8) -> Option<Entry> {
        if position >= self.count || position >= DESCRIPTORS {
            return None;
        }
        let index = (self.head + position) % DESCRIPTORS;
        Some(Entry {
            index,
            descriptor: self.descriptors[usize::from(index)],
        })
    }

    pub fn pop(&mut self) -> Option<Entry> {
        let entry = self.head()?;
        let descriptor = entry.descriptor;
        if descriptor.bytes > self.bytes_used || descriptor.handles > self.handles_used {
            return None;
        }
        self.descriptors[usize::from(entry.index)] = EMPTY;
        self.head = (self.head + 1) % DESCRIPTORS;
        self.count -= 1;
        self.bytes_head = (self.bytes_head + descriptor.bytes) % BYTES;
        self.bytes_used -= descriptor.bytes;
        self.handles_head = (self.handles_head + descriptor.handles) % HANDLES;
        self.handles_used -= descriptor.handles;
        Some(entry)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InboxRefusal {
    TooLarge,
    Full,
}

impl fmt::Display for InboxRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the message exceeds 2048 bytes or 16 handles",
            Self::Full => "the inbox has no room for the message",
        })
    }
}

impl core::error::Error for InboxRefusal {}
