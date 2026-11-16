use memory_object::{Extent, PLACEMENT_SAMPLES, Record, RecordRefusal, Records, place};
use native_abi::limits::PLACEMENT_START;
use wost::arch::current::paging::{AddressSpace, Flush, MapError, UserAccess};
use wost::mm::SharedFrame;
use wost::mm::pool::Earmark;
use x86_paging::{PAGE_SIZE, USER_END};

use crate::arena::{Local, Pages};
use crate::memory::MemoryObject;
use crate::object::ObjectRef;
use crate::thread::ThreadContext;
use crate::world::World;

const _: () = assert!(
    native_abi::limits::SPACE_RECORDS as usize == memory_object::RECORDS,
    "the ABI's record limit is the record set's"
);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Target {
    // The record holds a strong reference to the object, and a writer while it maps `Write`.
    Object {
        memory: Local<MemoryObject>,
        first: u32,
    },
    // A ring, buffer 0 or time page, whose frame the space keeps while it maps it.
    Shared(Page),
    Guard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    Ring,
    Data,
    Time,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mapping {
    pub(crate) access: UserAccess,
    pub(crate) target: Target,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MapRefusal {
    Taken,
    Full,
    Frames(MapError),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UnmapRefusal {
    NotFound,
    NotAnObject,
}

// A user address space and the records every present user leaf in it is the image of.
#[derive(Debug)]
pub(crate) struct Space {
    address: AddressSpace,
    seed: [u8; 32],
    counter: u64,
    records: Pages<Records<Mapping>>,
    shared: [Option<SharedFrame>; 3],
}

impl Space {
    pub(crate) fn frames_for() -> u32 {
        Pages::<Records<Mapping>>::frames_for(1)
    }

    // The records live in the world's arena, so the space itself stays small to move.
    pub(crate) fn new(address: AddressSpace, seed: [u8; 32], earmark: &mut Earmark) -> Self {
        Self {
            address,
            seed,
            counter: 0,
            records: Pages::build(earmark, 1, || Records::EMPTY),
            shared: [const { None }; 3],
        }
    }

    fn records(&mut self) -> &mut Records<Mapping> {
        let Some(records) = self.records.get_mut(0) else {
            panic!("a space lost its record page");
        };
        records
    }

    pub(crate) fn address(&mut self) -> &mut AddressSpace {
        &mut self.address
    }

    // A seeded address for `pages` pages recorded as `mapping`, above `guard` pages left unmapped.
    // It takes over the references the mapping carries and lets them go on a refusal.
    pub(crate) fn place(
        &mut self,
        world: &World,
        pages: u64,
        guard: u64,
        mapping: Mapping,
    ) -> Option<u64> {
        let placed = self.record_placed(pages, guard, mapping);
        if placed.is_none() {
            release(world, mapping);
        }
        placed
    }

    fn record_placed(&mut self, pages: u64, guard: u64, mapping: Mapping) -> Option<u64> {
        let range = Extent::new(
            PLACEMENT_START / PAGE_SIZE,
            (USER_END - PLACEMENT_START) / PAGE_SIZE,
        )?;
        let total = pages.checked_add(guard)?;
        let mut samples = [0u64; PLACEMENT_SAMPLES as usize];
        for (index, sample) in (0u64..).zip(samples.iter_mut()) {
            let index = self.counter.wrapping_add(index);
            let [a, b, c, d, e, f, g, h, ..] =
                entropy::derive(&self.seed, b"placement", index, wost::mm::wipe);
            *sample = u64::from_le_bytes([a, b, c, d, e, f, g, h]);
        }
        let counter = self.counter;
        let placed = place(self.records(), &samples, counter, range, total, 0).ok()?;
        let extent = Extent::new(placed.first + guard, pages)?;
        // Both records fit or neither is made, so a refusal leaves the set as it was.
        if self.records().count() + usize::from(guard != 0) + 1 > memory_object::RECORDS {
            return None;
        }
        if guard != 0 {
            let fence = Mapping {
                access: UserAccess::Read,
                target: Target::Guard,
            };
            self.records()
                .insert(Extent::new(placed.first, guard)?, fence)
                .ok()?;
        }
        self.records().insert(extent, mapping).ok()?;
        self.counter = placed.counter;
        Some(extent.first() * PAGE_SIZE)
    }

    // Records the mapping at `at` and maps its pages, its tables taken from `tables`. It takes over
    // the references the mapping carries; a refusal leaves no record and no leaf and lets them go.
    pub(crate) fn map(
        &mut self,
        context: &mut ThreadContext,
        world: &World,
        tables: &mut Earmark,
        at: u64,
        pages: u64,
        mapping: Mapping,
    ) -> Result<(), MapRefusal> {
        let inserted = Extent::new(at / PAGE_SIZE, pages)
            .ok_or(MapRefusal::Taken)
            .and_then(|extent| {
                self.records()
                    .insert(extent, mapping)
                    .map(|()| extent)
                    .map_err(|refusal| match refusal {
                        RecordRefusal::Full => MapRefusal::Full,
                        _ => MapRefusal::Taken,
                    })
            });
        let extent = match inserted {
            Ok(extent) => extent,
            Err(refusal) => {
                release(world, mapping);
                return Err(refusal);
            }
        };
        self.fill(context, world, tables, at).map_err(|reason| {
            let removed = self.records().remove(extent);
            assert!(removed.is_ok(), "a refused mapping lost its record");
            release(world, mapping);
            MapRefusal::Frames(reason)
        })
    }

    // Maps every page of the object record at `at`; a refusal clears what it mapped.
    pub(crate) fn fill(
        &mut self,
        context: &mut ThreadContext,
        world: &World,
        tables: &mut Earmark,
        at: u64,
    ) -> Result<(), MapError> {
        let Some((extent, access, memory, first)) = self.object_record(at) else {
            panic!("a space filled a record that names no object");
        };
        let Some(object) = world.memories.hold(memory) else {
            panic!("a mapping record outlived the object it holds");
        };
        let mut mapped = 0;
        let filled = object.frames(context, first, extent.pages(), |page, frame| {
            self.address
                .map_frame(at + page * PAGE_SIZE, frame, access, tables)?;
            mapped += 1;
            Ok(())
        });
        if filled.is_err() {
            let mut flush = Flush::none();
            let cleared = object.frames(context, first, mapped, |page, frame| {
                let cleared = self.address.clear_frame(at + page * PAGE_SIZE, frame)?;
                flush.merge(cleared);
                Ok::<(), MapError>(())
            });
            if let Err(reason) = cleared {
                panic!("a mapping written a moment ago refused to clear: {reason}");
            }
            drop(object);
            flush.wait(context);
        }
        filled
    }

    // Maps the one page of the shared record at `at`, its table taken from `tables`; the space keeps
    // the frame until it clears.
    pub(crate) fn share(
        &mut self,
        tables: &mut Earmark,
        at: u64,
        frame: SharedFrame,
    ) -> Result<(), MapError> {
        let record = Extent::new(at / PAGE_SIZE, 1)
            .and_then(|extent| self.records().covering(extent))
            .filter(|record| record.extent.first() * PAGE_SIZE == at && record.extent.pages() == 1);
        let Some(Record {
            value:
                Mapping {
                    access,
                    target: Target::Shared(page),
                },
            ..
        }) = record
        else {
            panic!("a space shared a page at {at:#x} with no shared record there");
        };
        let slot = &mut self.shared[page as usize];
        assert!(slot.is_none(), "a shared record was mapped twice");
        let frame = slot.insert(frame);
        self.address.map_frame(at, frame.frame(), access, tables)
    }

    // Whole object records only; the record's references go after the flush completes.
    pub(crate) fn unmap(
        &mut self,
        context: &mut ThreadContext,
        world: &World,
        at: u64,
        pages: u64,
    ) -> Result<(), UnmapRefusal> {
        let extent = Extent::new(at / PAGE_SIZE, pages).ok_or(UnmapRefusal::NotFound)?;
        let mapping = self
            .records()
            .covering(extent)
            .filter(|record| record.extent == extent)
            .ok_or(UnmapRefusal::NotFound)?
            .value;
        if !matches!(mapping.target, Target::Object { .. }) {
            return Err(UnmapRefusal::NotAnObject);
        }
        let removed = self.records().remove(extent);
        assert!(removed.is_ok(), "an unmapped record vanished");
        let flush = self.clear_record(context, world, extent, mapping);
        flush.wait(context);
        release(world, mapping);
        Ok(())
    }

    // Every object record is cleared, then released once the flush completes; the space is then
    // ready to be reclaimed.
    pub(crate) fn clear(&mut self, context: &mut ThreadContext, world: &World) {
        let mut flush = Flush::none();
        for position in 0..self.records().count() {
            if let Some(record) = self.records().get(position) {
                flush.merge(self.clear_record(context, world, record.extent, record.value));
            }
        }
        flush.wait(context);
        self.shared = [const { None }; 3];
        while let Some(record) = self.records().get(0) {
            let removed = self.records().remove(record.extent);
            assert!(removed.is_ok(), "a cleared record vanished");
            release(world, record.value);
        }
    }

    // A page never mapped (a spawn refused part way) has no leaf to clear.
    fn clear_record(
        &mut self,
        context: &mut ThreadContext,
        world: &World,
        extent: Extent,
        mapping: Mapping,
    ) -> Flush {
        let at = extent.first() * PAGE_SIZE;
        let (memory, first) = match mapping.target {
            Target::Object { memory, first } => (memory, first),
            Target::Shared(page) => {
                let Some(frame) = &self.shared[page as usize] else {
                    return Flush::none();
                };
                return match self.address.clear_frame(at, frame.frame()) {
                    Ok(flush) => flush,
                    Err(MapError::NotMapped(_)) => Flush::none(),
                    Err(reason) => panic!("a shared page refused to clear: {reason}"),
                };
            }
            Target::Guard => return Flush::none(),
        };
        let Some(object) = world.memories.hold(memory) else {
            panic!("a mapping record outlived the object it holds");
        };
        let mut flush = Flush::none();
        let cleared = object.frames(context, first, extent.pages(), |page, frame| {
            match self.address.clear_frame(at + page * PAGE_SIZE, frame) {
                Ok(cleared) => flush.merge(cleared),
                Err(MapError::NotMapped(_)) => {}
                Err(reason) => return Err(reason),
            }
            Ok(())
        });
        if let Err(reason) = cleared {
            panic!("a recorded mapping refused to clear: {reason}");
        }
        flush
    }

    fn object_record(&mut self, at: u64) -> Option<(Extent, UserAccess, Local<MemoryObject>, u32)> {
        let record = self.records().covering(Extent::new(at / PAGE_SIZE, 1)?)?;
        match record.value.target {
            Target::Object { memory, first } => {
                Some((record.extent, record.value.access, memory, first))
            }
            Target::Shared(_) | Target::Guard => None,
        }
    }
}

fn release(world: &World, mapping: Mapping) {
    let Target::Object { memory, .. } = mapping.target else {
        return;
    };
    if mapping.access == UserAccess::Write {
        let Some(object) = world.memories.hold(memory) else {
            panic!("a writable mapping outlived the object it holds");
        };
        object.release_writer();
    }
    ObjectRef::Memory(memory).release(world);
}
