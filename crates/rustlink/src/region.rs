//! Regions and cursors: bounds-checked link resolution.

use crate::{Dangling, Link, Target};

/// A contiguous array of `T` against which links are resolved.
///
/// Every resolution is bounds-checked, so a link applied to the wrong region, a corrupted
/// link or a dangling link yields `None` or `Err`, never undefined behavior.
pub trait Region<T> {
    /// The elements of the region.
    fn as_slice(&self) -> &[T];

    /// Number of elements.
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// True if the region has no elements.
    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// The element at `index`, if any.
    fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }

    /// Resolves `link`, owned by the element at `owner`. `None` if the link is null, invalid,
    /// or its target is outside the region.
    fn follow<L: Link<T>>(&self, owner: usize, link: L) -> Option<(usize, &T)> {
        match link.target(owner) {
            Target::Index(index) => self.get(index).map(|element| (index, element)),
            Target::Null | Target::Invalid => None,
        }
    }

    /// Resolves `link`, distinguishing null (`Ok(None)`) from a dangling target (`Err`).
    fn try_follow<L: Link<T>>(
        &self,
        owner: usize,
        link: L,
    ) -> Result<Option<(usize, &T)>, Dangling> {
        match link.target(owner) {
            Target::Null => Ok(None),
            Target::Index(index) => match self.get(index) {
                Some(element) => Ok(Some((index, element))),
                None => Err(Dangling { owner }),
            },
            Target::Invalid => Err(Dangling { owner }),
        }
    }

    /// A cursor at `index`, or `None` if `index` is outside the region.
    fn cursor(&self, index: usize) -> Option<Cursor<'_, T>> {
        let items = self.as_slice();
        (index < items.len()).then_some(Cursor { items, index })
    }
}

/// A position in a region: the element slice and the index of the current element.
///
/// The index is always in bounds, so [`Cursor::get`] never panics.
pub struct Cursor<'a, T> {
    items: &'a [T],
    index: usize,
}

impl<T> Clone for Cursor<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Cursor<'_, T> {}

impl<'a, T> Cursor<'a, T> {
    /// Index of the current element.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The current element.
    pub fn get(&self) -> &'a T {
        &self.items[self.index]
    }

    /// Follows a link stored in the current element. `None` if null, invalid or out of bounds.
    pub fn follow<L: Link<T>>(&self, link: L) -> Option<Cursor<'a, T>> {
        match link.target(self.index) {
            Target::Index(index) if index < self.items.len() => Some(Cursor {
                items: self.items,
                index,
            }),
            _ => None,
        }
    }

    /// Follows a link stored in the current element, distinguishing null from dangling.
    pub fn try_follow<L: Link<T>>(&self, link: L) -> Result<Option<Cursor<'a, T>>, Dangling> {
        match link.target(self.index) {
            Target::Null => Ok(None),
            Target::Index(index) if index < self.items.len() => Ok(Some(Cursor {
                items: self.items,
                index,
            })),
            Target::Index(_) | Target::Invalid => Err(Dangling { owner: self.index }),
        }
    }
}
