mod not;
mod or;
mod tracking;

use crate::component::Component;
use crate::entity_id::EntityId;
#[cfg(feature = "parallel")]
use crate::iter::ParShiperator;
use crate::iter::{captain::ShiperatorCaptain, mixed::Mixed, Shiperator};
use crate::optional::Optional;
use crate::sparse_set::{FullRawWindow, FullRawWindowMut, RawEntityIdAccess};
use crate::storage::StorageId;
use crate::tracking::Tracking;
use crate::views::{View, ViewMut};
use crate::ShipHashSet;
use alloc::vec::Vec;
use core::ptr::NonNull;

// A full plan also improves captain selection and parallel splitting, unlike
// an empty-only probe. Allow up to half the provisional driver's slots in
// metadata chunks per tracking input; do not impose a storage-size ceiling.
const PLANNING_BUDGET_DIVISOR: usize = 2;

#[inline]
fn planning_budget(driver_len: Option<usize>) -> usize {
    driver_len.map_or(usize::MAX, |len| {
        if len == 0 {
            0
        } else {
            (len / PLANNING_BUDGET_DIVISOR).max(1)
        }
    })
}

/// Creates view iterators.
///
/// `std::iter::IntoIterator` can't be used directly because of conflicting implementation.\
/// This trait serves as substitute.
pub trait IntoIter: IntoShiperator {
    /// ### Example
    /// ```
    /// use shipyard::{Component, EntitiesViewMut, IntoIter, ViewMut, World};
    ///
    /// #[derive(Component, Clone, Copy)]
    /// struct U32(u32);
    ///
    /// #[derive(Component)]
    /// struct USIZE(usize);
    ///
    /// let world = World::new();
    ///
    /// let (mut entities, mut usizes, mut u32s) = world.borrow::<(EntitiesViewMut, ViewMut<USIZE>, ViewMut<U32>)>().unwrap();
    ///
    /// entities.add_entity((&mut usizes, &mut u32s), (USIZE(0), U32(1)));
    /// entities.add_entity((&mut usizes, &mut u32s), (USIZE(2), U32(3)));
    ///
    /// (&mut usizes, &u32s).iter().for_each(|(mut x, &y)| {
    ///     x.0 += y.0 as usize;
    /// });
    /// ```
    fn iter(self) -> Shiperator<Self::Shiperator>;
    /// ### Example
    /// ```
    /// use rayon::prelude::ParallelIterator;
    /// use shipyard::{Component, EntitiesViewMut, IntoIter, ViewMut, World};
    ///
    /// #[derive(Component, Clone, Copy)]
    /// struct U32(u32);
    ///
    /// #[derive(Component)]
    /// struct USIZE(usize);
    ///
    /// let world = World::new();
    ///
    /// let (mut entities, mut usizes, mut u32s) = world.borrow::<(EntitiesViewMut, ViewMut<USIZE>, ViewMut<U32>)>().unwrap();
    ///
    /// entities.add_entity((&mut usizes, &mut u32s), (USIZE(0), U32(1)));
    /// entities.add_entity((&mut usizes, &mut u32s), (USIZE(2), U32(3)));
    ///
    /// (&mut usizes, &u32s).par_iter().for_each(|(mut x, &y)| {
    ///     x.0 += y.0 as usize;
    /// });
    /// ```
    #[cfg(feature = "parallel")]
    #[cfg_attr(docsrs, doc(cfg(feature = "parallel")))]
    fn par_iter(self) -> ParShiperator<Self::Shiperator>;
}

impl<T: IntoShiperator> IntoIter for T
where
    <T as IntoShiperator>::Shiperator: ShiperatorCaptain,
{
    #[inline]
    fn iter(self) -> Shiperator<Self::Shiperator> {
        let mut storage_ids = ShipHashSet::new();
        let (shiperator, mut len, mut entities) = self.into_shiperator(&mut storage_ids);
        if shiperator.has_no_candidates() {
            len = 0;
            entities = RawEntityIdAccess::dangling();
        }
        let is_infallible = shiperator.is_exact_sized();

        Shiperator {
            shiperator,
            is_exact_sized: is_infallible,
            entities,
            start: 0,
            end: len,
            min_split_len: 1,
        }
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn par_iter(self) -> ParShiperator<Self::Shiperator> {
        ParShiperator(self.iter())
    }
}

/// Turns a view into a Shiperator.
pub trait IntoShiperator {
    #[allow(missing_docs)]
    type Shiperator;

    /// Returns the Shiperator, its maximum length and `RawEntityIdAccess`.
    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess);
    /// Unfiltered driver length, available without inspecting tracking metadata.
    /// `None` leaves custom inputs on their existing construction path.
    #[inline]
    fn planning_len(&self) -> Option<usize> {
        None
    }
    /// Constructs an iterator with a per-input tracking metadata budget.
    /// Exceeding the budget must keep exact traversal, never prove emptiness.
    #[inline]
    fn into_shiperator_with_budget(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
        _max_chunks: usize,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess)
    where
        Self: Sized,
    {
        self.into_shiperator(storage_ids)
    }
    /// Returns `true` if the Shiperator can be a captain.
    fn can_captain() -> bool;
    /// Returns `true` if the Shiperator can be a sailor.
    fn can_sailor() -> bool;
}

impl<'tmp, 'v: 'tmp, T: Component, Track: Tracking> IntoShiperator for &'tmp View<'v, T, Track> {
    type Shiperator = FullRawWindow<'tmp, T>;

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        Some(self.len())
    }

    #[inline]
    fn into_shiperator(
        self,
        _storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let window = FullRawWindow::from_view(self);
        let len = window.len();
        let iter = window.entity_iter();

        (window, len, iter)
    }

    #[inline]
    fn can_captain() -> bool {
        true
    }

    #[inline]
    fn can_sailor() -> bool {
        true
    }
}

impl<'tmp, 'v: 'tmp, T: Component, Track: Tracking> IntoShiperator for &'tmp ViewMut<'v, T, Track> {
    type Shiperator = FullRawWindow<'tmp, T>;

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        Some(self.len())
    }

    #[inline]
    fn into_shiperator(
        self,
        _storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let window = FullRawWindow::from_view_mut(self);
        let len = window.len();
        let iter = window.entity_iter();

        (window, len, iter)
    }

    #[inline]
    fn can_captain() -> bool {
        true
    }

    #[inline]
    fn can_sailor() -> bool {
        true
    }
}

impl<'tmp, 'v: 'tmp, T: Component, Track> IntoShiperator for &'tmp mut ViewMut<'v, T, Track> {
    type Shiperator = FullRawWindowMut<'tmp, T, Track>;

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        Some(self.len())
    }

    #[inline]
    fn into_shiperator(
        self,
        _storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let window = FullRawWindowMut::new(self);
        let len = window.len();
        let iter = window.entity_iter();

        (window, len, iter)
    }

    #[inline]
    fn can_captain() -> bool {
        true
    }

    #[inline]
    fn can_sailor() -> bool {
        true
    }
}

impl<'tmp> IntoShiperator for &'tmp [EntityId] {
    type Shiperator = &'tmp [EntityId];

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        Some(self.len())
    }

    #[inline]
    fn into_shiperator(
        self,
        _storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let len = self.len();
        let iter =
            RawEntityIdAccess::new(NonNull::new(self.as_ptr().cast_mut()).unwrap(), Vec::new());

        (self, len, iter)
    }

    #[inline]
    fn can_captain() -> bool {
        true
    }

    #[inline]
    fn can_sailor() -> bool {
        false
    }
}

impl<'tmp, 'v: 'tmp, T: Component> IntoShiperator for Optional<&'tmp View<'v, T>> {
    type Shiperator = Optional<FullRawWindow<'tmp, T>>;

    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let (shiperator, len, entities) = self.0.into_shiperator(storage_ids);

        (Optional(shiperator), len, entities)
    }

    fn can_captain() -> bool {
        false
    }

    fn can_sailor() -> bool {
        true
    }
}

impl<'tmp, 'v: 'tmp, T: Component, Track: Tracking> IntoShiperator
    for Optional<&'tmp ViewMut<'v, T, Track>>
{
    type Shiperator = Optional<FullRawWindow<'tmp, T>>;

    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let (shiperator, len, entities) = self.0.into_shiperator(storage_ids);

        (Optional(shiperator), len, entities)
    }

    fn can_captain() -> bool {
        false
    }

    fn can_sailor() -> bool {
        true
    }
}

impl<'tmp, 'v: 'tmp, T: Component, Track: Tracking> IntoShiperator
    for Optional<&'tmp mut ViewMut<'v, T, Track>>
{
    type Shiperator = Optional<FullRawWindowMut<'tmp, T, Track>>;

    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let (shiperator, len, entities) = self.0.into_shiperator(storage_ids);

        (Optional(shiperator), len, entities)
    }

    fn can_captain() -> bool {
        false
    }

    fn can_sailor() -> bool {
        true
    }
}

impl<T: IntoShiperator> IntoShiperator for (T,) {
    type Shiperator = T::Shiperator;

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        self.0.planning_len()
    }

    #[inline]
    fn into_shiperator_with_budget(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
        max_chunks: usize,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        self.0.into_shiperator_with_budget(storage_ids, max_chunks)
    }

    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        self.0.into_shiperator(storage_ids)
    }

    fn can_captain() -> bool {
        T::can_captain()
    }

    fn can_sailor() -> bool {
        T::can_sailor()
    }
}

// It's not currently possible to have a final non repeating '+'
// https://github.com/rust-lang/rust/issues/18700
macro_rules! strip_plus {
    (+ $($rest: tt)*) => {
        $($rest)*
    }
}

// This is used in mixed.rs
pub(crate) use strip_plus;

macro_rules! impl_into_shiperator_tuple {
    ($(($type: ident, $index: tt))+) => {
        impl<$($type: IntoShiperator),+> IntoShiperator for ($($type,)+) where $(<$type as IntoShiperator>::Shiperator: ShiperatorCaptain),+ {
            type Shiperator = Mixed<($($type::Shiperator,)+)>;

            #[inline]
            fn planning_len(&self) -> Option<usize> {
                // An entity-id slice cannot be a sailor, so its length takes
                // precedence even when another component storage is smaller.
                $(if !$type::can_sailor() { return self.$index.planning_len(); })+
                let mut len: Option<usize> = None;
                $(
                    if $type::can_captain() {
                        if let Some(candidate) = self.$index.planning_len() {
                            len = Some(len.map_or(candidate, |current| current.min(candidate)));
                        }
                    }
                )+
                len
            }

            #[inline]
            #[track_caller]
            fn into_shiperator(
                self,
                storage_ids: &mut ShipHashSet<StorageId>,
            ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
                self.into_shiperator_with_budget(storage_ids, usize::MAX)
            }

            #[inline]
            #[track_caller]
            fn into_shiperator_with_budget(
                self,
                storage_ids: &mut ShipHashSet<StorageId>,
                max_chunks: usize,
            ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
                // Decide the budget before any child reads tracking metadata.
                // Nested joins may tighten an inherited budget, never expand it.
                let max_chunks = max_chunks.min(planning_budget(self.planning_len()));
                let mut shiperators = ($(self.$index.into_shiperator_with_budget(storage_ids, max_chunks),)+);

                let can_captains = ($(
                    $type::can_captain(),
                )+);

                let can_any_captain = $(
                    can_captains.$index
                )||+;

                if !can_any_captain {
                    panic!("Unable to build a Shiperator: None of the views could be a Captain.")
                }

                let can_sailors = ($(
                    $type::can_sailor(),
                )+);

                $(
                    if !can_captains.$index && !can_sailors.$index {
                        panic!("Unable to build a Shiperator: View at index {} could neither be a Captain nor a Sailor.", $index)
                    }
                )+

                let unable_sailor = strip_plus!($(
                    + (!can_sailors.$index as usize)
                )+);

                if unable_sailor > 1 {
                    panic!("Unable to build a Shiperator: Multiple views were unable to be Sailors.")
                }

                let sail_times = ($(
                    shiperators.$index.0.sail_time(),
                )+);

                if unable_sailor == 1 {
                    let mut mask = 0;
                    let mut len = 0;
                    let  mut entity_iter = RawEntityIdAccess::dangling();

                    for (index, (can_sailor, shiperator_len, shiperator_entity_iter)) in
                        [$((can_sailors.$index, shiperators.$index.1, shiperators.$index.2)),+]
                            .into_iter()
                            .enumerate()
                    {
                        if !can_sailor {
                            mask = 1 << index;
                            len = shiperator_len;
                            entity_iter = shiperator_entity_iter;

                            break;
                        }
                    }

                    $(
                        if mask & (1 << $index) == 0 {
                            shiperators.$index.0.unpick();
                        }
                    )+

                    return (
                        Mixed {
                            shiperator: ($(shiperators.$index.0,)+),
                            mask
                        },
                        len,
                        entity_iter,
                    );
                }

                let mut mask = 0;
                let mut len = 0;
                let mut entity_iter = RawEntityIdAccess::dangling();
                let mut min_sail_time = usize::MAX;

                $(
                    if can_captains.$index && sail_times.$index < min_sail_time {
                        mask = 1 << $index;
                        len = shiperators.$index.1;
                        entity_iter = shiperators.$index.2;
                        min_sail_time = sail_times.$index;
                    }
                )+

                let _ = min_sail_time;

                $(
                    if mask & (1 << $index) == 0 {
                        shiperators.$index.0.unpick();
                    }
                )+

                (
                    Mixed {
                        shiperator: ($(shiperators.$index.0,)+),
                        mask,
                    },
                    len,
                    entity_iter,
                )
            }

            #[inline]
            fn can_captain() -> bool {
                $(
                    $type::can_captain()
                )||+
            }

            #[inline]
            fn can_sailor() -> bool {
                $(
                    $type::can_sailor()
                )&&+
            }
        }
    };
}

macro_rules! into_shiperator_tuple {
    ($(($type: ident, $index: tt))+; ($type1: ident, $index1: tt) $(($queue_type: ident, $queue_index: tt))*) => {
        impl_into_shiperator_tuple![$(($type, $index))*];
        into_shiperator_tuple![$(($type, $index))* ($type1, $index1); $(($queue_type, $queue_index))*];
    };
    ($(($type: ident, $index: tt))+;) => {
        impl_into_shiperator_tuple![$(($type, $index))*];
    }
}

#[cfg(not(feature = "extended_tuple"))]
into_shiperator_tuple![(A, 0) (B, 1); (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)];
#[cfg(feature = "extended_tuple")]
into_shiperator_tuple![
    (A, 0) (B, 1); (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)
    (K, 10) (L, 11) (M, 12) (N, 13) (O, 14) (P, 15) (Q, 16) (R, 17) (S, 18) (T, 19)
    (U, 20) (V, 21) (W, 22) (X, 23) (Y, 24) (Z, 25) (AA, 26) (BB, 27) (CC, 28) (DD, 29)
    (EE, 30) (FF, 31)
];
