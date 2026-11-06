use std::fmt::{Display, Formatter};
use std::mem::swap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TextMut {
    storage: String,
}

impl Default for TextMut {
    fn default() -> Self {
        Self::new()
    }
}

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

impl Deref for TextMut {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.storage
    }
}

impl DerefMut for TextMut {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage
    }
}

impl TextMut {
    pub const fn new() -> Self {
        Self {
            storage: String::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            storage: String::with_capacity(capacity),
        }
    }

    pub fn freeze(self) -> TextRef<'static> {
        TextRef::Shared(Arc::new(self.storage))
    }
}

#[derive(Clone, Debug)]
pub enum TextRef<'a> {
    Ref(&'a str),
    Shared(Arc<String>),
}

impl<'a> AsRef<str> for TextRef<'a> {
    fn as_ref(&self) -> &str {
        match self {
            TextRef::Ref(v) => *v,
            TextRef::Shared(v) => v.as_str(),
        }
    }
}

impl<'a> Deref for TextRef<'a> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a> Display for TextRef<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <str as Display>::fmt(self.as_ref(), f)
    }
}

impl<'a> TextRef<'a> {
    pub fn edit(self) -> TextMut {
        let storage = match self {
            TextRef::Ref(v) => v.to_string(),
            TextRef::Shared(v) => Arc::try_unwrap(v).unwrap_or_else(|v| v.as_ref().clone()),
        };
        TextMut { storage }
    }

    pub fn edit_in_place<F, R>(&mut self, modification: F) -> R
    where F: FnOnce(&mut String) -> R
    {
        let mut tmp = TextRef::Ref("");
        swap(&mut tmp, self);

        let mut tmp = tmp.edit();
        let ret = modification(&mut tmp);

        *self = tmp.freeze();
        ret
    }
}
