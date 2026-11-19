use crate::component::Component;
use crate::iter::planner::ConstructionMetadata;
use crate::iter::{IntoShiperator, PlannedTracking, RawEntityIdAccess};
use crate::sparse_set::{FullRawWindow, FullRawWindowMut};
use crate::tracking::{
    Inserted, InsertedOrModified, InsertionOrModificationTracking, InsertionTracking,
    ModificationTracking, Modified,
};
use crate::views::{View, ViewMut};

macro_rules! impl_into_shiperator_tracking {
    ($($type: ident => $capability: ident, $inserted: literal, $modified: literal;)+) => {$(
        // SAFE: We wrap the view's own results and mark them filtered, they never drive a grouped
        // walk. The filter only rejects candidates of the view's window, which upholds the rest.
        unsafe impl<'tmp, 'v: 'tmp, T: Component, Track: $capability> IntoShiperator
            for $type<&'tmp View<'v, T, Track>>
        {
            type Shiperator = PlannedTracking<$type<FullRawWindow<'tmp, T, Track>>>;

            #[inline]
            fn planning_len(&self) -> Option<usize> { Some(self.0.len()) }

            #[inline]
            fn into_shiperator(self) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                self.into_shiperator_with_budget(usize::MAX)
            }

            #[inline]
            fn into_shiperator_with_budget(
                self, max_chunks: usize,
            ) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                let (window, entities, _) = self.0.into_shiperator();

                let len = window.dense_len;
                let plan = window.tracking_plan::<$inserted, $modified>(max_chunks);
                (PlannedTracking { inner: $type(window), plan, len }, entities, ConstructionMetadata::filtered())
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

        // SAFE: We wrap the view's own results and mark them filtered, they never drive a grouped
        // walk. The filter only rejects candidates of the view's window, which upholds the rest.
        unsafe impl<'tmp, 'v: 'tmp, T: Component, Track: $capability> IntoShiperator
            for $type<&'tmp ViewMut<'v, T, Track>>
        {
            type Shiperator = PlannedTracking<$type<FullRawWindow<'tmp, T, Track>>>;

            #[inline]
            fn planning_len(&self) -> Option<usize> { Some(self.0.len()) }

            #[inline]
            fn into_shiperator(self) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                self.into_shiperator_with_budget(usize::MAX)
            }

            #[inline]
            fn into_shiperator_with_budget(
                self, max_chunks: usize,
            ) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                let (window, entities, _) = self.0.into_shiperator();

                let len = window.dense_len;
                let plan = window.tracking_plan::<$inserted, $modified>(max_chunks);
                (PlannedTracking { inner: $type(window), plan, len }, entities, ConstructionMetadata::filtered())
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

        // SAFE: We wrap the view's own results and mark them filtered, they never drive a grouped
        // walk. The filter only rejects candidates of the view's window, which upholds the rest.
        unsafe impl<'tmp, 'v: 'tmp, T: Component, Track: $capability> IntoShiperator
            for $type<&'tmp mut ViewMut<'v, T, Track>>
        {
            type Shiperator = PlannedTracking<$type<FullRawWindowMut<'tmp, T, Track>>>;

            #[inline]
            fn planning_len(&self) -> Option<usize> { Some(self.0.len()) }

            #[inline]
            fn into_shiperator(self) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                self.into_shiperator_with_budget(usize::MAX)
            }

            #[inline]
            fn into_shiperator_with_budget(
                self, max_chunks: usize,
            ) -> (Self::Shiperator, RawEntityIdAccess, ConstructionMetadata) {
                let (window, entities, _) = self.0.into_shiperator();

                let len = window.dense_len;
                let plan = window.tracking_plan::<$inserted, $modified>(max_chunks);
                (PlannedTracking { inner: $type(window), plan, len }, entities, ConstructionMetadata::filtered())
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
    )+};
}

impl_into_shiperator_tracking![
    Inserted => InsertionTracking, true, false;
    Modified => ModificationTracking, false, true;
    InsertedOrModified => InsertionOrModificationTracking, true, true;
];
