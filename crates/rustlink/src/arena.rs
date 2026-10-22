//! The append-only arena.

use alloc::vec::Vec;
use core::fmt;

use crate::Region;

/// An append-only array of `T` addressed by index.
///
/// There is no removal, insertion or reordering, so the index of an element never changes
/// and links held inside the arena never dangle. `clear` is the exception: it invalidates
/// every index at once.
///
/// # Examples
///
/// ```
/// use rustlink::{Arena, OptAbs, Region};
///
/// #[derive(Clone, Copy)]
/// struct Node {
///     value: u32,
///     next: OptAbs<Node, u16>,
/// }
///
/// let mut arena = Arena::new();
/// let tail = arena.push(Node { value: 2, next: OptAbs::NULL });
/// let head = arena.push(Node { value: 1, next: OptAbs::new(Some(tail)).unwrap() });
///
/// let first = arena.cursor(head).unwrap();
/// let second = first.follow(first.get().next).unwrap();
/// assert_eq!(second.get().value, 2);
/// ```
///
/// A link only resolves in a region of the element type it was made for:
///
/// ```compile_fail,E0277
/// use rustlink::{Abs, Arena, Region};
///
/// struct A;
/// struct B;
///
/// let arena: Arena<B> = Arena::new();
/// let link: Abs<A, u16> = Abs::new(0).unwrap();
/// let _ = arena.follow(0, link);
/// ```
pub struct Arena<T> {
    items: Vec<T>,
}

impl<T> Arena<T> {
    /// An empty arena.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// An empty arena with room for `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
        }
    }

    /// Appends `value` and returns its index.
    pub fn push(&mut self, value: T) -> usize {
        self.items.push(value);
        self.items.len() - 1
    }

    /// Mutable access to the element at `index`, for patching links after construction.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index)
    }

    /// Removes every element. All indices become invalid.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Consumes the arena and returns its elements.
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.items).finish()
    }
}

impl<T> Region<T> for Arena<T> {
    fn as_slice(&self) -> &[T] {
        &self.items
    }
}
