use super::planning_budget;
use crate::iter::IntoShiperator;
use crate::or::{Or, OrWindow};
use crate::sparse_set::RawEntityIdAccess;
use crate::storage::StorageId;
use crate::ShipHashSet;

impl<T: IntoShiperator, U: IntoShiperator> IntoShiperator for Or<(T, U)> {
    type Shiperator = OrWindow<(T::Shiperator, U::Shiperator)>;

    #[inline]
    fn planning_len(&self) -> Option<usize> {
        // OR traverses both sources; its first slice alone is not a driver bound.
        let (left, right) = &self.0;
        Some(left.planning_len()?.saturating_add(right.planning_len()?))
    }

    #[inline]
    fn into_shiperator(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        self.into_shiperator_with_budget(storage_ids, usize::MAX)
    }

    #[inline]
    fn into_shiperator_with_budget(
        self,
        storage_ids: &mut ShipHashSet<StorageId>,
        max_chunks: usize,
    ) -> (Self::Shiperator, usize, RawEntityIdAccess) {
        let max_chunks = max_chunks.min(planning_budget(self.planning_len()));
        let (left, right) = self.0;
        let (shiperator1, len1, entity_access1) =
            left.into_shiperator_with_budget(storage_ids, max_chunks);
        let (shiperator2, len2, entity_access2) =
            right.into_shiperator_with_budget(storage_ids, max_chunks);

        let left_slices = 1 + entity_access1.follow_up_ptrs.len();
        // Pending sources are a stack: left's remaining sources are visited
        // before right's first source and its own remaining sources.
        let mut follow_up = entity_access2.follow_up_ptrs;
        follow_up.reserve(left_slices);
        follow_up.push((entity_access2.ptr, len2));
        follow_up.extend(entity_access1.follow_up_ptrs);
        let entity_access = RawEntityIdAccess::new(entity_access1.ptr, follow_up);

        (
            OrWindow {
                storages: (shiperator1, shiperator2),
                left_slices,
                current_slice: 0,
            },
            len1,
            entity_access,
        )
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
