mod directory;
mod pool;

use core::fmt;
use core::marker::PhantomData;

use object_pool::Index;

pub(crate) use directory::Pages;
pub(crate) use pool::Pool;

// Names an object in the same world by page ordinal, slot and generation, never by address.
pub(crate) struct Local<T> {
    index: Index,
    generation: u32,
    object: PhantomData<fn() -> T>,
}

impl<T> Local<T> {
    const fn new(index: Index, generation: u32) -> Self {
        Self {
            index,
            generation,
            object: PhantomData,
        }
    }

    const fn index(self) -> Index {
        self.index
    }

    const fn generation(self) -> u32 {
        self.generation
    }

    pub(crate) fn to_bits(self) -> u64 {
        u64::from(self.index.bits()) << u32::BITS | u64::from(self.generation)
    }

    pub(crate) fn from_bits(bits: u64) -> Self {
        Self::new(Index::from_bits((bits >> u32::BITS) as u32), bits as u32)
    }
}

impl<T> Clone for Local<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Local<T> {}

impl<T> PartialEq for Local<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Local<T> {}

impl<T> fmt::Debug for Local<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Local")
            .field("page", &self.index.page())
            .field("slot", &self.index.slot())
            .field("generation", &self.generation)
            .finish()
    }
}
