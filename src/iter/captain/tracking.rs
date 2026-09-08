use crate::component::Component;
use crate::iter::ShiperatorCaptain;
use crate::sparse_set::{FullRawWindow, FullRawWindowMut, TRACKING_CHUNK_SIZE};
use crate::track;
use crate::tracking::{Inserted, InsertedOrModified, Modified};

macro_rules! impl_shiperator_captain_tracking {
    ($wrapper: ident, $check_insertion: expr, $check_modification: expr; $($track: path)+) => {
        impl<'tmp, T: Component> ShiperatorCaptain for $wrapper<FullRawWindow<'tmp, T>> {
            #[inline]
            fn has_stable_membership(&self) -> bool { true }
            #[inline]
            unsafe fn get_captain_data(&self, _index: usize) -> Self::Out {
                unreachable!()
            }

            #[inline]
            fn next_slice(&mut self) {}

            #[inline]
            fn sail_time(&self) -> usize {
                self.0.sail_time()
            }

            #[inline]
            fn is_exact_sized(&self) -> bool {
                false
            }

            #[inline]
            fn unpick(&mut self) {
                self.0.unpick();
            }

            #[inline]
            fn next_possible(&self, index: usize) -> usize {
                self.0
                    .next_tracked(index, self.0.len(), $check_insertion, $check_modification)
            }

            #[inline]
            fn next_possible_in(&self, index: usize, end: usize) -> usize {
                self.0.next_tracked(index, end, $check_insertion, $check_modification)
            }

            #[inline]
            fn is_definitely_empty(&self, max_chunks: usize) -> bool {
                if self.0.len().div_ceil(TRACKING_CHUNK_SIZE) > max_chunks {
                    return false;
                }
                self.0.next_tracked(0, self.0.len(), $check_insertion, $check_modification) >= self.0.len()
            }
        }

        $(
            impl<'tmp, T: Component> ShiperatorCaptain for $wrapper<FullRawWindowMut<'tmp, T, $track>> {
                #[inline]
                fn has_stable_membership(&self) -> bool { !$check_modification }
                #[inline]
                unsafe fn get_captain_data(&self, _index: usize) -> Self::Out {
                    unreachable!()
                }

                #[inline]
                fn next_slice(&mut self) {}

                #[inline]
                    fn sail_time(&self) -> usize {
                    self.0.sail_time()
                }

                #[inline]
                fn is_exact_sized(&self) -> bool {
                    false
                }

                #[inline]
                fn unpick(&mut self) {
                    self.0.unpick();
                }

                #[inline]
                fn next_possible(&self, index: usize) -> usize {
                    self.0
                        .next_tracked(index, self.0.len(), $check_insertion, $check_modification)
                }

                #[inline]
                fn next_possible_in(&self, index: usize, end: usize) -> usize {
                    self.0.next_tracked(index, end, $check_insertion, $check_modification)
                }

                #[inline]
                fn is_definitely_empty(&self, max_chunks: usize) -> bool {
                    if self.0.len().div_ceil(TRACKING_CHUNK_SIZE) > max_chunks {
                        return false;
                    }
                    self.0.next_tracked(0, self.0.len(), $check_insertion, $check_modification) >= self.0.len()
                }
            }
        )+
    };
}

impl_shiperator_captain_tracking![Inserted, true, false; track::Untracked track::Insertion track::InsertionAndDeletion track::InsertionAndRemoval track::InsertionAndDeletionAndRemoval track::Deletion track::DeletionAndRemoval track::Removal track::Modification track::InsertionAndModification track::InsertionAndModificationAndDeletion track::InsertionAndModificationAndRemoval track::ModificationAndDeletion track::ModificationAndRemoval track::ModificationAndDeletionAndRemoval track::All];
impl_shiperator_captain_tracking![Modified, false, true; track::Untracked track::Insertion track::InsertionAndDeletion track::InsertionAndRemoval track::InsertionAndDeletionAndRemoval track::Deletion track::DeletionAndRemoval track::Removal track::Modification track::InsertionAndModification track::InsertionAndModificationAndDeletion track::InsertionAndModificationAndRemoval track::ModificationAndDeletion track::ModificationAndRemoval track::ModificationAndDeletionAndRemoval track::All];
impl_shiperator_captain_tracking![InsertedOrModified, true, true; track::Untracked track::Insertion track::InsertionAndDeletion track::InsertionAndRemoval track::InsertionAndDeletionAndRemoval track::Deletion track::DeletionAndRemoval track::Removal track::Modification track::InsertionAndModification track::InsertionAndModificationAndDeletion track::InsertionAndModificationAndRemoval track::ModificationAndDeletion track::ModificationAndRemoval track::ModificationAndDeletionAndRemoval track::All];
