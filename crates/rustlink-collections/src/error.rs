//! The error returned when a link width cannot address another node.

use core::fmt;

/// A container is full: its link width cannot address one more node.
///
/// Carries back the value that was not inserted, so nothing is lost. Returned by the `try_`
/// methods; the plain methods panic with the same message instead.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// The value that did not fit.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Full(..)")
    }
}

impl<T> fmt::Display for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MESSAGE)
    }
}

impl<T> core::error::Error for Full<T> {}

/// The panic message of the plain (non-`try_`) insertion methods.
pub(crate) const MESSAGE: &str = "the container's link width cannot address another node";
