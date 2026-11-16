use core::fmt;

// The head and tail of a singly linked FIFO whose links live in the receivers themselves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Receivers<Id> {
    head: Option<Id>,
    tail: Option<Id>,
}

// The caller writes `next` into the link of `of`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Relink<Id> {
    pub of: Id,
    pub next: Option<Id>,
}

impl<Id: Copy + PartialEq> Receivers<Id> {
    pub const EMPTY: Self = Self {
        head: None,
        tail: None,
    };

    pub fn head(self) -> Option<Id> {
        self.head
    }

    // `id` must be unlinked, with its own link already empty.
    #[must_use]
    pub fn push(self, id: Id) -> (Self, Option<Relink<Id>>) {
        let mut relink = None;
        if let Some(tail) = self.tail {
            relink = Some(Relink {
                of: tail,
                next: Some(id),
            });
        }
        let head = if self.head.is_none() {
            Some(id)
        } else {
            self.head
        };
        (
            Self {
                head,
                tail: Some(id),
            },
            relink,
        )
    }

    // `after_head` is the link the head holds.
    #[must_use]
    pub fn pop(self, after_head: Option<Id>) -> (Self, Option<Id>) {
        let Some(head) = self.head else {
            return (self, None);
        };
        if self.tail == Some(head) {
            return (Self::EMPTY, Some(head));
        }
        (
            Self {
                head: after_head,
                tail: self.tail,
            },
            Some(head),
        )
    }

    // `before` is the receiver whose link names `id`, or `None` when `id` is the head.
    pub fn unlink(
        self,
        id: Id,
        before: Option<Id>,
        after: Option<Id>,
    ) -> Result<(Self, Option<Relink<Id>>), NotArmed> {
        let Some(head) = self.head else {
            return Err(NotArmed);
        };
        let relink = match before {
            None if head == id => None,
            None => return Err(NotArmed),
            Some(before) if before == id => return Err(NotArmed),
            Some(before) => Some(Relink {
                of: before,
                next: after,
            }),
        };
        let head = if before.is_none() { after } else { Some(head) };
        let tail = if self.tail == Some(id) {
            before
        } else {
            self.tail
        };
        Ok((Self { head, tail }, relink))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotArmed;

impl fmt::Display for NotArmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the receiver is not armed on this endpoint")
    }
}

impl core::error::Error for NotArmed {}
