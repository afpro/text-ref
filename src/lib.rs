//! Copy, edit and share text without duplicating its storage.
//!
//! The crate is built around two complementary types:
//!
//! * [`TextMut`] — uniquely owned, mutable text (a thin [`String`] wrapper).
//!   Use it while building or changing a buffer.
//! * [`TextRef`] — an immutable snapshot of text that can be cloned cheaply.
//!   Internally it is either a borrowed `&str` ([`TextRef::Ref`]) or a buffer
//!   shared through an atomic reference count ([`TextRef::Shared`]).
//!
//! Freezing a [`TextMut`] moves its buffer into an [`Arc`] without copying;
//! cloning a [`TextRef`] afterwards only bumps a reference count (or copies a
//! pointer for the borrowed variant). Editing a snapshot is explicit and
//! copy-on-write: [`TextRef::edit`] reuses the existing allocation when the
//! snapshot is the last owner and clones the text only when it is not.
//!
//! ```text
//!                    freeze()                        edit()
//!   TextMut  -------------------->  TextRef  <-------------------->  TextMut
//!  (unique, mutable)             (shared, immutable)             (unique, mutable)
//! ```
//!
//! # Examples
//!
//! ```
//! use text_ref::{TextMut, TextRef};
//!
//! // Build up some text ...
//! let mut builder = TextMut::from("hello");
//! builder.push_str(", world");
//!
//! // ... then freeze it into a snapshot. The buffer is moved, not copied.
//! let snapshot: TextRef<'static> = builder.freeze();
//!
//! // Cloning the snapshot is O(1): both handles point at the same buffer.
//! let copy = snapshot.clone();
//! assert_eq!(&*snapshot, "hello, world");
//! assert_eq!(&*copy, "hello, world");
//!
//! // Editing a snapshot copies the text once and leaves the snapshot intact.
//! let mut editable = copy.edit();
//! editable.push('!');
//! assert_eq!(editable.as_str(), "hello, world!");
//! assert_eq!(&*snapshot, "hello, world");
//! ```

use std::fmt::{Display, Formatter};
use std::mem::swap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Mutable, uniquely owned text.
///
/// `TextMut` is a thin wrapper around [`String`] that can be turned into a
/// cheaply shareable [`TextRef`] with [`TextMut::freeze`]. It dereferences to
/// [`String`], so every [`String`] method is available directly:
///
/// ```
/// use text_ref::TextMut;
///
/// let mut text = TextMut::from("hello");
/// text.push_str(", world");
/// assert_eq!(text.as_str(), "hello, world");
/// ```
#[derive(Clone, Debug)]
pub struct TextMut {
    /// The owned UTF-8 buffer.
    ///
    /// Kept private so that the only ways to reach it are the [`Deref`] and
    /// [`DerefMut`] impls (which borrow it) or [`TextMut::freeze`] (which
    /// moves it into shared, immutable storage).
    storage: String,
}

impl Default for TextMut {
    fn default() -> Self {
        Self::new()
    }
}

/// Creates a `TextMut` from anything that can be converted into a [`String`],
/// such as `&str`, [`String`] or `Cow<str>`.
impl<S> From<S> for TextMut
where
    String: From<S>,
{
    fn from(value: S) -> Self {
        Self {
            storage: value.into(),
        }
    }
}

/// Gives read access to the underlying [`String`] through `&self`.
impl Deref for TextMut {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.storage
    }
}

/// Gives mutable access to the underlying [`String`] through `&mut self`,
/// which is what makes `TextMut` the editable half of the crate.
impl DerefMut for TextMut {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage
    }
}

impl TextMut {
    /// Creates an empty `TextMut` without allocating.
    ///
    /// ```
    /// use text_ref::TextMut;
    ///
    /// let text = TextMut::new();
    /// assert!(text.is_empty());
    /// ```
    pub const fn new() -> Self {
        Self {
            storage: String::new(),
        }
    }

    /// Creates an empty `TextMut` with room for at least `capacity` bytes.
    ///
    /// Use this when the final size is roughly known, to avoid repeated
    /// reallocations while the text is being built.
    ///
    /// ```
    /// use text_ref::TextMut;
    ///
    /// let text = TextMut::with_capacity(1024);
    /// assert!(text.capacity() >= 1024);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            storage: String::with_capacity(capacity),
        }
    }

    /// Converts this owned text into an immutable, shareable snapshot.
    ///
    /// The [`String`] buffer is *moved* into an [`Arc`], so no byte of text is
    /// copied; only the small `Arc` bookkeeping is allocated. Because `self`
    /// is consumed, the returned [`TextRef`] does not borrow anything from the
    /// caller and can therefore live for `'static` (it is effectively an owned
    /// value that happens to be immutable).
    ///
    /// ```
    /// use text_ref::{TextMut, TextRef};
    ///
    /// let text = TextMut::from("hello");
    /// let snapshot: TextRef<'static> = text.freeze();
    ///
    /// // Two snapshots, one buffer.
    /// let another = snapshot.clone();
    /// assert_eq!(&*another, "hello");
    /// ```
    pub fn freeze(self) -> TextRef<'static> {
        // `self.storage` is moved into the `Arc`, so freezing is free of any
        // text copy regardless of how long the text is.
        TextRef::Shared(Arc::new(self.storage))
    }
}

/// An immutable text snapshot that can be cloned cheaply.
///
/// A `TextRef` is either a plain borrow of a `&str` ([`TextRef::Ref`]) or a
/// shared heap buffer ([`TextRef::Shared`]). In both cases cloning is cheap:
/// the borrowed variant copies a fat pointer, the shared variant increments a
/// reference count. Text is never copied when snapshots are cloned or passed
/// around.
///
/// The shared variant stores [`Arc<String>`] rather than `Arc<str>` so that
/// [`TextRef::edit`] can take the buffer back without copying when this
/// snapshot is the sole owner.
///
/// A snapshot never changes. To modify the text, turn it back into an owned
/// [`TextMut`] with [`TextRef::edit`] (or use [`TextRef::edit_in_place`]),
/// which copies the text only if someone else still holds it.
///
/// `TextRef` dereferences to [`str`], so it can be used almost anywhere a
/// string slice is expected.
///
/// ```
/// use text_ref::{TextMut, TextRef};
///
/// let snapshot = TextMut::from("shared").freeze();
/// let another = snapshot.clone(); // O(1): only the refcount changes
///
/// assert_eq!(&*snapshot, "shared");
/// assert_eq!(another.len(), 6);
/// ```
#[derive(Clone, Debug)]
pub enum TextRef<'a> {
    /// Text borrowed from somewhere else; cloning copies only the reference.
    Ref(&'a str),
    /// Text owned by a reference-counted buffer; cloning bumps the refcount.
    Shared(Arc<String>),
}

/// Borrows the snapshot as a `&str`, whichever variant it is.
impl<'a> AsRef<str> for TextRef<'a> {
    fn as_ref(&self) -> &str {
        match self {
            // Deref coercion turns the matched `&&str` into `&str`.
            TextRef::Ref(v) => v,
            // Deref the `Arc<String>` to `String`, then borrow it as `str`.
            TextRef::Shared(v) => v.as_str(),
        }
    }
}

/// Lets snapshots be used like `&str` (indexing, `split`, `trim`, ...).
impl<'a> Deref for TextRef<'a> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

/// Lets a snapshot be printed with `{}` directly.
impl<'a> Display for TextRef<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <str as Display>::fmt(self.as_ref(), f)
    }
}

impl<'a> TextRef<'a> {
    /// Turns the snapshot into uniquely owned, mutable text.
    ///
    /// This is the copy-on-write step:
    ///
    /// * For [`TextRef::Ref`] the borrowed bytes are copied into a fresh
    ///   [`String`].
    /// * For [`TextRef::Shared`] the [`Arc`] is unwrapped and its buffer reused
    ///   when this snapshot is the last owner; the text is cloned only while
    ///   other snapshots are still alive.
    ///
    /// The snapshot is consumed, so any clones that already exist keep
    /// pointing at the old, unchanged text.
    ///
    /// ```
    /// use text_ref::{TextMut, TextRef};
    ///
    /// let snapshot = TextMut::from("hello").freeze();
    ///
    /// // Still shared: `edit` has to clone the buffer once.
    /// let mut editable = snapshot.clone().edit();
    /// editable.push_str(", world");
    ///
    /// assert_eq!(&*snapshot, "hello");
    /// assert_eq!(editable.as_str(), "hello, world");
    /// ```
    pub fn edit(self) -> TextMut {
        let storage = match self {
            // Borrowed text: the only way to obtain an owned buffer is a copy.
            TextRef::Ref(v) => v.to_string(),
            // Shared text: reuse the allocation if we are the last owner.
            // `try_unwrap` returns the `Arc` back on failure, in which case
            // another snapshot still needs the buffer, so clone its contents.
            TextRef::Shared(v) => Arc::try_unwrap(v).unwrap_or_else(|v| v.as_ref().clone()),
        };
        TextMut { storage }
    }

    /// Edits the text through a closure and stores the result back in `self`.
    ///
    /// The snapshot's text is moved out into a temporary [`TextMut`], handed
    /// to `modification` as `&mut String`, then frozen back into `self`. The
    /// closure's return value is forwarded to the caller, so the edit can
    /// report something computed while it ran.
    ///
    /// This avoids repeating the edit, mutate, re-freeze dance at every call
    /// site. While the closure runs, `self` temporarily holds an empty string
    /// and cannot be accessed, because the `&mut self` borrow lasts for the
    /// whole call.
    ///
    /// # Panics
    ///
    /// If `modification` panics, `self` is left as an empty [`TextRef::Ref`];
    /// its previous content is dropped while unwinding.
    ///
    /// ```
    /// use text_ref::{TextMut, TextRef};
    ///
    /// let mut text: TextRef<'static> = TextMut::from("hello").freeze();
    ///
    /// let len = text.edit_in_place(|s| {
    ///     s.push_str(", world");
    ///     s.len() // returned to the caller
    /// });
    ///
    /// assert_eq!(len, 12);
    /// assert_eq!(&*text, "hello, world");
    /// ```
    pub fn edit_in_place<F, R>(&mut self, modification: F) -> R
    where
        F: FnOnce(&mut String) -> R,
    {
        // Move the text out of `self` (leaving a valid empty snapshot behind)
        // so the closure can own it mutably without borrowing `self` at all.
        let mut tmp = TextRef::Ref("");
        swap(&mut tmp, self);

        // Unshare the text if needed; this reuses the buffer when possible.
        let mut tmp = tmp.edit();
        let ret = modification(&mut tmp);

        // Share the edited buffer again and put it back in place.
        *self = tmp.freeze();
        ret
    }
}
