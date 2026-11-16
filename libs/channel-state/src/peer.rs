use crate::Unexpected;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link<Id> {
    Open(Id),
    PeerClosed,
    Forwarded(Id),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hop {
    First,
    Forwarded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route<Id> {
    Deliver,
    Follow(Id),
    PeerClosed,
}

impl<Id: Copy + PartialEq> Link<Id> {
    pub fn close(self) -> Result<Self, Unexpected> {
        match self {
            Self::Open(_) => Ok(Self::PeerClosed),
            Self::PeerClosed | Self::Forwarded(_) => Err(Unexpected),
        }
    }

    pub fn forward(self, to: Id) -> Result<Self, Unexpected> {
        match self {
            Self::Open(_) => Ok(Self::Forwarded(to)),
            Self::PeerClosed | Self::Forwarded(_) => Err(Unexpected),
        }
    }

    pub fn repoint(self, peer: Id) -> Result<Self, Unexpected> {
        match self {
            Self::Open(_) => Ok(Self::Open(peer)),
            Self::PeerClosed | Self::Forwarded(_) => Err(Unexpected),
        }
    }
}

// `receiver` is the link of the endpoint the sender's own link names; delivery needs it to name
// the sender back, and a forward is followed once.
pub fn route<Id: Copy + PartialEq>(sender: Id, receiver: Link<Id>, hop: Hop) -> Route<Id> {
    match (receiver, hop) {
        (Link::Open(peer), _) if peer == sender => Route::Deliver,
        (Link::Forwarded(to), Hop::First) => Route::Follow(to),
        _ => Route::PeerClosed,
    }
}
