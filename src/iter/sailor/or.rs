use crate::entity_id::EntityId;
use crate::iter::{ShiperatorCaptain, ShiperatorSailor};
use crate::or::{OneOfTwo, OrWindow};

impl<T: ShiperatorCaptain + ShiperatorSailor, U: ShiperatorCaptain + ShiperatorSailor>
    ShiperatorSailor for OrWindow<(T, U)>
{
    type Index = OneOfTwo<T::Index, U::Index>;

    #[inline]
    unsafe fn get_sailor_data(&self, index: Self::Index) -> Self::Out {
        match index {
            OneOfTwo::One(index) => OneOfTwo::One((self.storages).0.get_sailor_data(index)),
            OneOfTwo::Two(index) => OneOfTwo::Two((self.storages).1.get_sailor_data(index)),
        }
    }

    #[inline]
    fn indices_of(&self, eid: EntityId, index: usize) -> Option<Self::Index> {
        // Membership probes must not depend on which source drives iteration.
        // This also preserves left precedence without probing right on a hit.
        if let Some(index) = self.storages.0.indices_of(eid, index) {
            Some(OneOfTwo::One(index))
        } else {
            self.storages.1.indices_of(eid, index).map(OneOfTwo::Two)
        }
    }

    #[inline]
    fn captain_indices_of(&self, eid: EntityId, index: usize) -> Option<Self::Index> {
        if self.current_slice < self.left_slices {
            let index = self.storages.0.captain_indices_of(eid, index)?;
            if !self.storages.0.has_stable_membership() {
                // A returned mutable value can stop matching the left filter.
                // Remember it so a later right source cannot yield it again.
                self.seen_left
                    .borrow_mut()
                    .get_or_insert_with(Default::default)
                    .insert(eid);
            }
            Some(OneOfTwo::One(index))
        } else {
            let index = self.storages.1.captain_indices_of(eid, index)?;
            let seen_left = self
                .seen_left
                .borrow()
                .as_ref()
                .is_some_and(|seen| seen.contains(&eid));
            // Also check current membership for reverse and mixed traversal.
            if seen_left || self.storages.0.indices_of(eid, 0).is_some() {
                return None;
            }
            Some(OneOfTwo::Two(index))
        }
    }

    #[inline]
    fn index_from_usize(_index: usize) -> Self::Index {
        unreachable!()
    }
}
