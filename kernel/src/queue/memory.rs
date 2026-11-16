use core::ops::Deref;

use memory_object::{Seal, StateRefusal};
use native_abi::Rights;
use native_abi::ring::{Completion, Submission, flags, status};
use wost::mm::pool;
use wost::mm::slab::Hold;

use super::{Process, answer, refuse};
use crate::arena::Local;
use crate::memory::{Kind, MemoryObject, SEALED_RIGHTS};
use crate::object::ObjectRef;
use crate::world::{MemoryPool, World};

// The largest run of a copy held on the stack at once.
const COPY_CHUNK: usize = 512;

pub(super) fn create(process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    let kind = match submission.flags {
        0 => Kind::Sealable,
        flags::SHAREABLE => Kind::Shareable,
        _ => return refuse(token, status::DENIED),
    };
    let Some(pages) = u32::try_from(submission.argument)
        .ok()
        .filter(|pages| *pages != 0)
    else {
        return refuse(token, status::DENIED);
    };
    let Some(frames) = MemoryPool::frames_for(1)
        .zip(MemoryObject::frames_for(pages))
        .and_then(|(record, contents)| record.checked_add(contents))
    else {
        return refuse(token, status::NO_MEMORY);
    };
    let Ok(reservation) = process.table_ref.reserve_slot() else {
        return refuse(token, status::NO_HANDLE_SLOT);
    };
    let Ok(mut earmark) = pool::earmark(process.context, process.world.arena(), frames) else {
        return refuse(token, status::NO_MEMORY);
    };
    let object = MemoryObject::new(&mut earmark, kind, pages);
    let object = ObjectRef::Memory(process.world.memories.insert(&mut earmark, object));
    drop(earmark);
    let rights = kind.rights();
    match reservation.install(process.context, object, rights, rights) {
        Ok(handle) => answer(token, handle),
        Err(_) => {
            object.release(process.world);
            refuse(token, status::BAD_HANDLE)
        }
    }
}

// Through a bounce buffer, so the two objects' locks are never held together.
pub(super) fn copy(mut process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    let target = match resolve(&mut process, submission.handle, Rights::WRITE) {
        Ok(target) => target,
        Err(status) => return refuse(token, status),
    };
    let source = match resolve(&mut process, submission.second, Rights::READ) {
        Ok(source) => source,
        Err(status) => return refuse(token, status),
    };
    let (Some(target), Some(source)) = (
        Held::take(process.world, target),
        Held::take(process.world, source),
    ) else {
        return refuse(token, status::BAD_HANDLE);
    };
    let len = submission.length as usize;
    let (into, from) = (submission.offset, submission.argument);
    let in_range = |object: &MemoryObject, at: u64| {
        at.checked_add(len as u64)
            .is_some_and(|end| end <= object.bytes())
    };
    if !in_range(&target, into) || !in_range(&source, from) {
        return refuse(token, status::BAD_BUFFER);
    }
    match target.acquire_writer() {
        Ok(()) => {}
        Err(StateRefusal::Saturated) => return refuse(token, status::BUSY),
        Err(_) => return refuse(token, status::DENIED),
    }
    let mut chunk = [0u8; COPY_CHUNK];
    let mut done = 0;
    while done < len {
        let run = &mut chunk[..COPY_CHUNK.min(len - done)];
        let offset = done as u64;
        assert!(
            source.read(process.context, from + offset, run)
                && target.write(process.context, into + offset, run),
            "a copy inside both objects' bounds was refused"
        );
        done += run.len();
    }
    target.release_writer();
    answer(token, 0)
}

// The object's single handle becomes one new handle without `WRITE`; a refusal leaves both alone.
pub(super) fn seal(mut process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    if submission.flags != flags::SEAL_READ || submission.second != 0 {
        return refuse(token, status::DENIED);
    }
    let local = match resolve(&mut process, submission.handle, Rights::WRITE) {
        Ok(local) => local,
        Err(status) => return refuse(token, status),
    };
    let Some(object) = Held::take(process.world, local) else {
        return refuse(token, status::BAD_HANDLE);
    };
    if object.kind() != Kind::Sealable || !process.table.renewable(submission.handle) {
        return refuse(token, status::DENIED);
    }
    match object.seal(process.context, Seal::Read) {
        Ok(()) => {}
        Err(StateRefusal::Busy) => return refuse(token, status::BUSY),
        Err(_) => return refuse(token, status::DENIED),
    }
    drop(object);
    // A close racing the seal took the handle first: the object it named is sealed and gone.
    match process.table.replace(
        process.context,
        submission.handle,
        ObjectRef::Memory(local),
        SEALED_RIGHTS,
    ) {
        Ok(handle) => answer(token, handle),
        Err(_) => refuse(token, status::BAD_HANDLE),
    }
}

fn resolve(
    process: &mut Process<'_, '_>,
    handle: u64,
    wanted: Rights,
) -> Result<Local<MemoryObject>, u32> {
    let Ok((object, rights)) = process.table.resolve(process.context, handle) else {
        return Err(status::BAD_HANDLE);
    };
    let ObjectRef::Memory(local) = object else {
        return Err(status::DENIED);
    };
    if !rights.contains(wanted) {
        return Err(status::DENIED);
    }
    Ok(local)
}

// A strong reference for the operation's length, so a close racing it leaves the last release here.
struct Held<'world> {
    world: &'world World,
    local: Local<MemoryObject>,
    hold: Option<Hold<MemoryObject>>,
}

impl<'world> Held<'world> {
    fn take(world: &'world World, local: Local<MemoryObject>) -> Option<Self> {
        world.memories.acquire(local).ok()?;
        let held = Self {
            world,
            local,
            hold: world.memories.hold(local),
        };
        held.hold.is_some().then_some(held)
    }
}

impl Deref for Held<'_> {
    type Target = MemoryObject;

    fn deref(&self) -> &MemoryObject {
        let Some(hold) = self.hold.as_ref() else {
            panic!("a held memory object lost its hold");
        };
        hold
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        drop(self.hold.take());
        ObjectRef::Memory(self.local).release(self.world);
    }
}
