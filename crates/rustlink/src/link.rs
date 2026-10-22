//! The four link types and the [`Link`] trait.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::{LinkError, Width};

/// What a link decodes to, before any bounds check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The link is null.
    Null,
    /// The link refers to this element index.
    Index(usize),
    /// The decoded index is outside `0..=usize::MAX`. Only relative links can produce this.
    Invalid,
}

/// A link to an element of type `T`.
pub trait Link<T>: Copy {
    /// Decodes the link. `owner` is the index of the element that stores the link; absolute
    /// links ignore it.
    fn target(self, owner: usize) -> Target;

    /// True if the link is null, or its target is a valid index into a region of `len` elements.
    fn check(self, owner: usize, len: usize) -> bool {
        match self.target(owner) {
            Target::Null => true,
            Target::Index(i) => i < len,
            Target::Invalid => false,
        }
    }
}

macro_rules! link_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned)]
        #[repr(C)]
        pub struct $name<T, W: Width>(W::Le, PhantomData<fn() -> T>);

        impl<T, W: Width> Clone for $name<T, W> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<T, W: Width> Copy for $name<T, W> {}

        impl<T, W: Width> PartialEq for $name<T, W> {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl<T, W: Width> Eq for $name<T, W> {}

        impl<T, W: Width> Hash for $name<T, W> {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
    };
}

link_type! {
    /// An absolute link: the index of the target element. Never null.
    ///
    /// The width bounds the region size: `Abs<T, u16>` can address 65,536 elements.
    Abs
}

link_type! {
    /// An optional absolute link. Stores `index + 1`; stored 0 is null.
    ///
    /// Addresses one element fewer than [`Abs`] of the same width.
    OptAbs
}

link_type! {
    /// A relative link: the signed element delta `target - owner`, where `owner` is the index of
    /// the element that stores the link. Never null; delta 0 is the owner itself.
    ///
    /// The width bounds the link distance, not the region size.
    Rel
}

link_type! {
    /// An optional relative link. Stored delta 0 is null, so a non-null link cannot point at
    /// its own owner.
    OptRel
}

impl<T, W: Width> Abs<T, W> {
    /// Creates a link to element `index`.
    pub fn new(index: usize) -> Result<Self, LinkError> {
        let v = index as u64;
        if v > W::MAX {
            return Err(LinkError::OutOfRange {
                value: index as i128,
                min: 0,
                max: i128::from(W::MAX),
            });
        }
        Ok(Self(W::encode(v), PhantomData))
    }

    /// The target index. Saturates to `usize::MAX` if the stored value exceeds `usize`.
    pub fn index(self) -> usize {
        usize::try_from(W::decode(self.0)).unwrap_or(usize::MAX)
    }
}

impl<T, W: Width> OptAbs<T, W> {
    /// The null link.
    pub const NULL: Self = Self(W::ZERO, PhantomData);

    /// Creates a link to element `index`, or the null link for `None`.
    pub fn new(index: Option<usize>) -> Result<Self, LinkError> {
        let Some(i) = index else {
            return Ok(Self::NULL);
        };
        match (i as u64).checked_add(1) {
            Some(stored) if stored <= W::MAX => Ok(Self(W::encode(stored), PhantomData)),
            _ => Err(LinkError::OutOfRange {
                value: i as i128,
                min: 0,
                max: i128::from(W::MAX) - 1,
            }),
        }
    }

    /// The target index, or `None` for null. Saturates to `usize::MAX` if the stored value
    /// exceeds `usize`.
    pub fn get(self) -> Option<usize> {
        match W::decode(self.0) {
            0 => None,
            stored => Some(usize::try_from(stored - 1).unwrap_or(usize::MAX)),
        }
    }
}

impl<T, W: Width> Default for OptAbs<T, W> {
    fn default() -> Self {
        Self::NULL
    }
}

fn delta_between<W: Width>(owner: usize, target: usize) -> Result<i64, LinkError> {
    let delta = target as i128 - owner as i128;
    let (min, max) = (i128::from(W::SIGNED_MIN), i128::from(W::SIGNED_MAX));
    if delta < min || delta > max {
        return Err(LinkError::OutOfRange {
            value: delta,
            min,
            max,
        });
    }
    Ok(delta as i64)
}

impl<T, W: Width> Rel<T, W> {
    /// Creates a link from the element at `owner` to the element at `target`.
    pub fn new(owner: usize, target: usize) -> Result<Self, LinkError> {
        Ok(Self(
            W::encode_signed(delta_between::<W>(owner, target)?),
            PhantomData,
        ))
    }

    /// The stored delta.
    pub fn delta(self) -> i64 {
        W::decode_signed(self.0)
    }
}

impl<T, W: Width> OptRel<T, W> {
    /// The null link.
    pub const NULL: Self = Self(W::ZERO, PhantomData);

    /// Creates a link from the element at `owner` to the element at `target`, or the null link
    /// for `None`. Fails with [`LinkError::NullCollision`] if `target == owner`.
    pub fn new(owner: usize, target: Option<usize>) -> Result<Self, LinkError> {
        match target {
            None => Ok(Self::NULL),
            Some(t) if t == owner => Err(LinkError::NullCollision),
            Some(t) => Ok(Self(
                W::encode_signed(delta_between::<W>(owner, t)?),
                PhantomData,
            )),
        }
    }

    /// The stored delta, or `None` for null.
    pub fn delta(self) -> Option<i64> {
        match W::decode_signed(self.0) {
            0 => None,
            delta => Some(delta),
        }
    }
}

impl<T, W: Width> Default for OptRel<T, W> {
    fn default() -> Self {
        Self::NULL
    }
}

fn relative_target(owner: usize, delta: i64) -> Target {
    match usize::try_from(owner as i128 + i128::from(delta)) {
        Ok(index) => Target::Index(index),
        Err(_) => Target::Invalid,
    }
}

impl<T, W: Width> Link<T> for Abs<T, W> {
    fn target(self, _owner: usize) -> Target {
        Target::Index(self.index())
    }
}

impl<T, W: Width> Link<T> for OptAbs<T, W> {
    fn target(self, _owner: usize) -> Target {
        match self.get() {
            None => Target::Null,
            Some(index) => Target::Index(index),
        }
    }
}

impl<T, W: Width> Link<T> for Rel<T, W> {
    fn target(self, owner: usize) -> Target {
        relative_target(owner, self.delta())
    }
}

impl<T, W: Width> Link<T> for OptRel<T, W> {
    fn target(self, owner: usize) -> Target {
        match self.delta() {
            None => Target::Null,
            Some(delta) => relative_target(owner, delta),
        }
    }
}

impl<T, W: Width> fmt::Debug for Abs<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Abs({})", self.index())
    }
}

impl<T, W: Width> fmt::Debug for OptAbs<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OptAbs({:?})", self.get())
    }
}

impl<T, W: Width> fmt::Debug for Rel<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rel({})", self.delta())
    }
}

impl<T, W: Width> fmt::Debug for OptRel<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OptRel({:?})", self.delta())
    }
}

const fn has_layout<L>(bytes: usize) -> bool {
    size_of::<L>() == bytes && align_of::<L>() == 1
}

macro_rules! assert_layouts {
    ($($link:ident),*) => {
        $(
            const _: () = {
                assert!(has_layout::<$link<(), u8>>(1));
                assert!(has_layout::<$link<(), u16>>(2));
                assert!(has_layout::<$link<(), u32>>(4));
                assert!(has_layout::<$link<(), u64>>(8));
            };
        )*
    };
}

assert_layouts!(Abs, OptAbs, Rel, OptRel);
