// SPDX-License-Identifier: GPL-3.0-or-later

//! Generational slot arena backing the entity stores (dirs, files, and in
//! part 2 users and actions), standing in for Gource's raw `new`/`delete`
//! pointers.
//!
//! An id is `(index, generation)`. Removing an entity bumps the slot's
//! generation and puts the slot on a LIFO free list, so a stale id never
//! resolves to the entity that later reuses the slot ([`Arena::get`] returns
//! `None`; indexing panics). Slot reuse is deterministic: it depends only on
//! the sequence of inserts and removes.

use std::marker::PhantomData;

/// Typed entity id. `Ord` is index-then-generation; it is the deterministic
/// "id order" used where Gource iterates a `std::set` of pointers.
pub(crate) struct Id<T> {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Id {
            index,
            generation,
            _marker: PhantomData,
        }
    }
}

impl<T> Copy for Id<T> {}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}

impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> std::fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}.{}", self.index, self.generation)
    }
}

#[derive(Clone, Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Clone, Debug)]
pub(crate) struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }
}

impl<T> Arena<T> {
    pub(crate) fn insert(&mut self, value: T) -> Id<T> {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return Id::new(index, slot.generation);
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Id::new(index, 0)
    }

    pub(crate) fn remove(&mut self, id: Id<T>) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        self.len -= 1;
        Some(value)
    }

    pub(crate) fn get(&self, id: Id<T>) -> Option<&T> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_ref()
    }

    pub(crate) fn get_mut(&mut self, id: Id<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_mut()
    }

    pub(crate) fn contains(&self, id: Id<T>) -> bool {
        self.get(id).is_some()
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Remove everything. Generations survive, so ids from before the clear
    /// stay stale.
    pub(crate) fn clear(&mut self) {
        self.free.clear();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.value.take().is_some() {
                slot.generation = slot.generation.wrapping_add(1);
            }
            self.free.push(index as u32);
        }
        // Reuse low slots first, like a fresh arena would.
        self.free.reverse();
        self.len = 0;
    }

    /// Live entities in slot order.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value
                .as_ref()
                .map(|v| (Id::new(i as u32, slot.generation), v))
        })
    }
}

impl<T> Arena<T> {
    /// Live entities in slot order, mutably.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| {
            let generation = slot.generation;
            slot.value
                .as_mut()
                .map(|v| (Id::new(i as u32, generation), v))
        })
    }
}

impl<T> std::ops::Index<Id<T>> for Arena<T> {
    type Output = T;

    fn index(&self, id: Id<T>) -> &T {
        self.get(id).expect("stale or invalid entity id")
    }
}

impl<T> std::ops::IndexMut<Id<T>> for Arena<T> {
    fn index_mut(&mut self, id: Id<T>) -> &mut T {
        self.get_mut(id).expect("stale or invalid entity id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_do_not_resolve() {
        let mut a: Arena<&str> = Arena::default();
        let x = a.insert("x");
        let y = a.insert("y");
        assert_eq!(a.remove(x), Some("x"));
        assert_eq!(a.get(x), None);
        let z = a.insert("z");
        assert_eq!(z.index, x.index);
        assert_ne!(z, x);
        assert_eq!(a[z], "z");
        assert_eq!(a[y], "y");
        assert_eq!(a.len(), 2);
        a.clear();
        assert!(a.is_empty());
        assert_eq!(a.get(y), None);
        let w = a.insert("w");
        assert_eq!(w.index, 0);
    }
}
