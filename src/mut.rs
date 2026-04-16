use crate::tracking::TrackingTimestamp;

/// Tracks component modification.
pub struct Mut<'a, T: ?Sized> {
    pub(crate) flag: Option<&'a mut TrackingTimestamp>,
    pub(crate) current: TrackingTimestamp,
    pub(crate) data: &'a mut T,
}

/// Tracks component modification through explicit read/write methods.
///
/// Unlike [`Mut`], this type does not implement `DerefMut`.
/// Use [`AsRef::as_ref`] for reads, [`AsMut::as_mut`] or [`SafeMut::modify`]
/// for tracked writes, and [`SafeMut::modify_without_tracking`] for intentional
/// untracked writes.
pub struct SafeMut<'a, T: ?Sized> {
    inner: Mut<'a, T>,
}

impl<'a, T: ?Sized> Mut<'a, T> {
    /// Makes a new [`Mut`], the component will not be flagged if its modified inside `f`.
    ///
    /// This is an associated function that needs to be used as `Mut::map(...)`. A method would interfere with methods of the same name used through Deref.
    pub fn map<U: ?Sized, F: FnOnce(&mut T) -> &mut U>(orig: Self, f: F) -> Mut<'a, U> {
        Mut {
            flag: orig.flag,
            current: orig.current,
            data: f(orig.data),
        }
    }
}

impl<'a, T: ?Sized> SafeMut<'a, T> {
    pub(crate) fn new(inner: Mut<'a, T>) -> Self {
        Self { inner }
    }

    /// Runs `f` with mutable access and marks the component as modified.
    pub fn modify<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        f(self.inner.as_mut())
    }

    /// Runs `f` with mutable access without marking the component as modified.
    ///
    /// This is intended for cleanup paths that consume already-observed data and
    /// should not wake `modified` systems again on the next tick.
    pub fn modify_without_tracking<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        f(self.inner.data)
    }
}

impl<'a, T: ?Sized> From<Mut<'a, T>> for SafeMut<'a, T> {
    fn from(inner: Mut<'a, T>) -> Self {
        Self::new(inner)
    }
}

impl<T: ?Sized> core::ops::Deref for Mut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T: ?Sized> core::ops::Deref for SafeMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.data
    }
}

impl<T: ?Sized> core::ops::DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        if let Some(flag) = &mut self.flag {
            **flag = self.current;
        }

        self.data
    }
}

impl<T: ?Sized> AsRef<T> for Mut<'_, T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self.data
    }
}

impl<T: ?Sized> AsRef<T> for SafeMut<'_, T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self.inner.as_ref()
    }
}

impl<T: ?Sized> AsMut<T> for Mut<'_, T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        if let Some(flag) = &mut self.flag {
            **flag = self.current;
        }

        self.data
    }
}

impl<T: ?Sized> AsMut<T> for SafeMut<'_, T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self.inner.as_mut()
    }
}

impl<T: ?Sized + core::fmt::Debug> core::fmt::Debug for SafeMut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: ?Sized + core::fmt::Debug> core::fmt::Debug for Mut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.data.fmt(f)
    }
}
