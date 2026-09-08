mod not;
mod or;
mod tracking;

use crate::component::Component;
use crate::entity_id::EntityId;
use crate::iter::ShiperatorOutput;
use crate::optional::Optional;
use crate::r#mut::{ModFlag, Mut, SafeMut};
use crate::sparse_set::{FullRawWindow, FullRawWindowMut};
use crate::track;

/// Provides access to components when the storage drives the iteration.
pub trait ShiperatorCaptain: ShiperatorOutput {
    /// Returns the component at `index`.
    ///
    /// # Safety
    ///
    /// `index` must be less than `end`, the length given by `into_shiperator` by this Shiperator.
    unsafe fn get_captain_data(&self, index: usize) -> Self::Out;
    /// Shiperators might iterate multiple splices of `EntityId`s.\
    /// This function is called on the switch to the next slice.
    fn next_slice(&mut self);
    /// Selects a source slice without consuming another source's cursor.
    /// Single-source iterators keep the default implementation.
    #[inline]
    fn set_slice(&mut self, _slice: usize) {}
    /// Current source index in the captain's concatenated entity slices.
    #[inline]
    fn slice_index(&self) -> usize {
        0
    }
    /// Approximation of how much time iterating this Shiperator will take.\
    /// This helps pick the fastest Shiperator when iterating multiple storages.
    ///
    /// Iterating a `Vec` of lenght 100 will return 100.
    fn sail_time(&self) -> usize;
    /// `true` when this Shiperator cannot return `None`.
    fn is_exact_sized(&self) -> bool;
    /// Conservatively proves that this required iterator cannot yield an item.
    /// At most `max_chunks` tracking metadata chunks may be inspected per member.
    /// The default keeps existing traversal for custom and non-AND wrappers.
    /// Retained for compatibility. Iterator construction uses `has_no_candidates`
    /// and does not call this method.
    #[inline]
    fn is_definitely_empty(&self, _max_chunks: usize) -> bool {
        false
    }
    /// Whether membership probes can run concurrently with mutable outputs.
    /// OR deduplication may probe a different producer's source. The default
    /// conservatively keeps such custom inputs on one producer.
    #[inline]
    fn has_stable_membership(&self) -> bool {
        false
    }
    /// Whether splitting this query can race with cross-source membership tests.
    #[inline]
    fn can_split(&self) -> bool {
        true
    }
    /// By default `into_shiperator` returns Shiperators that thinks they are captains.\
    /// This function is called on the ones that end up not being picked.
    fn unpick(&mut self);
    /// True only when current metadata proves that this required input is empty.
    #[inline]
    fn has_no_candidates(&self) -> bool {
        false
    }
    /// Upper bound on candidate slots in this captain's dense-index interval.
    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_count(&self, start: usize, end: usize) -> usize {
        end - start
    }
    /// Candidate slots in a particular source, including pending OR sources.
    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_count_at(&self, _slice: usize, start: usize, end: usize) -> usize {
        self.candidate_count(start, end)
    }
    /// Split a nonempty interval near half of its candidate work.
    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_midpoint(&self, start: usize, end: usize) -> usize {
        start + (end - start) / 2
    }
    /// Returns the next index at or after `index` that may yield an item.
    ///
    /// Tracking Shiperators use it to skip chunks without any flagged component.
    #[inline]
    fn next_possible(&self, index: usize) -> usize {
        index
    }
    /// Returns the next candidate in `index..end`, or `end` if none remains.
    /// Tracking captains stop scanning at the producer's exclusive upper bound.
    /// Custom captains retain their existing `next_possible` implementation.
    #[inline]
    fn next_possible_in(&self, index: usize, end: usize) -> usize {
        if index >= end {
            return end;
        }
        self.next_possible(index).min(end)
    }
    /// Exclusive upper bound at or before `end` whose last slot is a candidate.
    /// Returns zero when no preceding slot can match.
    #[inline]
    fn previous_possible(&self, end: usize) -> usize {
        end
    }
}

impl<'tmp, T: Component> ShiperatorCaptain for FullRawWindow<'tmp, T> {
    #[inline]
    fn has_stable_membership(&self) -> bool {
        true
    }
    #[inline]
    unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
        &*self.data.add(index)
    }

    #[inline]
    fn next_slice(&mut self) {}

    #[inline]
    fn sail_time(&self) -> usize {
        self.dense_len
    }

    #[inline]
    fn is_exact_sized(&self) -> bool {
        true
    }

    #[inline]
    fn unpick(&mut self) {}
}

macro_rules! impl_shiperator_captain_no_mut {
    ($($track: path)+) => {
        $(
            impl<'tmp, T: Component> ShiperatorCaptain for FullRawWindowMut<'tmp, T, $track> {
                #[inline]
                fn has_stable_membership(&self) -> bool { true }
                #[inline]
                unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
                    &mut *self.data.add(index)
                }

                #[inline]
                fn next_slice(&mut self) {}

                #[inline]
                fn sail_time(&self) -> usize {
                    self.dense_len
                }

                #[inline]
                fn is_exact_sized(&self) -> bool {
                    true
                }

                #[inline]
                fn unpick(&mut self) {}
            }
        )+
    }
}

impl_shiperator_captain_no_mut![track::Untracked track::Insertion track::InsertionAndDeletion track::InsertionAndRemoval track::InsertionAndDeletionAndRemoval track::Deletion track::DeletionAndRemoval track::Removal];

macro_rules! impl_shiperator_captain_mut {
    ($($track: path)+) => {
        $(
            impl<'tmp, T: Component> ShiperatorCaptain for FullRawWindowMut<'tmp, T, $track> {
                #[inline]
                fn has_stable_membership(&self) -> bool { true }
                #[inline]
                unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
                    SafeMut::new(Mut {
                        flag: Some(ModFlag {
                            slot: &mut *self.modification_data.add(index),
                            chunk: self.modification_chunk(index),
                        }),
                        current: self.current,
                        data: &mut *self.data.add(index),
                    })
                }

                #[inline]
                fn next_slice(&mut self) {}

                #[inline]
                fn sail_time(&self) -> usize {
                    self.dense_len
                }

                #[inline]
                fn is_exact_sized(&self) -> bool {
                    true
                }

                #[inline]
                fn unpick(&mut self) {}
            }
        )+
    }
}

impl_shiperator_captain_mut![track::Modification track::InsertionAndModification track::InsertionAndModificationAndDeletion track::InsertionAndModificationAndRemoval track::ModificationAndDeletion track::ModificationAndRemoval track::ModificationAndDeletionAndRemoval track::All];

impl<'tmp> ShiperatorCaptain for &'tmp [EntityId] {
    #[inline]
    fn has_stable_membership(&self) -> bool {
        true
    }
    unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
        *self.get_unchecked(index)
    }

    fn next_slice(&mut self) {}

    fn sail_time(&self) -> usize {
        self.len()
    }

    fn is_exact_sized(&self) -> bool {
        false
    }

    fn unpick(&mut self) {}
}

impl<'tmp, T: Component> ShiperatorCaptain for Optional<FullRawWindow<'tmp, T>> {
    #[inline]
    fn has_stable_membership(&self) -> bool {
        true
    }
    unsafe fn get_captain_data(&self, _index: usize) -> Self::Out {
        unreachable!()
    }

    fn next_slice(&mut self) {}

    fn sail_time(&self) -> usize {
        self.0.sail_time()
    }

    fn is_exact_sized(&self) -> bool {
        false
    }

    fn unpick(&mut self) {}
}

impl<'tmp, T: Component, Track> ShiperatorCaptain for Optional<FullRawWindowMut<'tmp, T, Track>>
where
    Optional<FullRawWindowMut<'tmp, T, Track>>: ShiperatorOutput,
    FullRawWindowMut<'tmp, T, Track>: ShiperatorCaptain,
{
    #[inline]
    fn has_stable_membership(&self) -> bool {
        true
    }
    unsafe fn get_captain_data(&self, _index: usize) -> Self::Out {
        unreachable!()
    }

    fn next_slice(&mut self) {}

    fn sail_time(&self) -> usize {
        self.0.sail_time()
    }

    fn is_exact_sized(&self) -> bool {
        false
    }

    fn unpick(&mut self) {}
}
