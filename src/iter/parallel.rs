use crate::entity_id::EntityId;
use crate::iter::{Shiperator, ShiperatorCaptain, ShiperatorSailor, WithId};
use crate::sparse_set::RawEntityIdAccess;
use alloc::vec::Vec;
use core::num::NonZeroUsize;

const MIN_SPLIT_LEN: usize = 16;

#[allow(missing_docs)]
pub struct ParShiperator<S>(pub(crate) Shiperator<S>, pub(crate) Option<NonZeroUsize>);

impl<S> ParShiperator<S> {
    /// Returns the [`EntityId`] alongside the component(s).
    pub fn with_id(self) -> WithId<Self> {
        WithId(self)
    }

    /// Sets the candidate-count threshold at which this iterator stops splitting.
    ///
    /// A producer with at most `threshold` remaining candidate slots will not
    /// split. Candidate slots are an upper bound before filtering and OR
    /// deduplication, not the number of results or the dense-index span.
    /// Rayon may stop splitting earlier, and queries that cannot safely split
    /// still use a single producer. This is not a maximum task size.
    ///
    /// This replaces the automatic threshold for this iterator only. Without
    /// an override, it remains `max(candidates / (threads * 4), 16)`, using the
    /// current Rayon pool when the iterator is consumed. Repeated calls use
    /// the last value, which is preserved by [`Self::with_id`].
    ///
    /// # Panics
    ///
    /// Panics if `threshold` is zero.
    ///
    /// # Example
    ///
    /// ```
    /// use rayon::prelude::*;
    /// use shipyard::{track, Component, IntoIter, View, World};
    ///
    /// struct Value(usize);
    /// impl Component for Value {
    ///     type Tracking = track::Untracked;
    /// }
    ///
    /// let mut world = World::new();
    /// world.add_entity(Value(3));
    /// let values = world.borrow::<View<Value>>()?;
    /// let sum: usize = values.par_iter().with_split_threshold(8).map(|v| v.0).sum();
    /// assert_eq!(sum, 3);
    /// # Ok::<(), shipyard::error::GetStorage>(())
    /// ```
    pub fn with_split_threshold(mut self, threshold: usize) -> Self {
        self.1 =
            Some(NonZeroUsize::new(threshold).expect("split threshold must be greater than zero"));
        self
    }
}

fn configure_split<S: ShiperatorCaptain>(
    producer: &mut Shiperator<S>,
    threshold: Option<NonZeroUsize>,
) {
    if let Some(threshold) = threshold {
        producer.min_split_len = threshold.get();
        return;
    }
    let total_len = producer
        .shiperator
        .candidate_count(producer.start, producer.end)
        .saturating_add(producer.pending_candidate_count());
    let threads = rayon::current_num_threads().max(1);

    producer.min_split_len = (total_len / (threads * 4)).max(MIN_SPLIT_LEN);
}

impl<S: ShiperatorCaptain> Shiperator<S> {
    fn pending_candidate_count(&self) -> usize {
        let mut count = 0usize;
        let first = self.shiperator.slice_index() + 1;
        for (offset, &(_, len)) in self.entities.follow_up_ptrs.iter().rev().enumerate() {
            count =
                count.saturating_add(self.shiperator.candidate_count_at(first + offset, 0, len));
        }
        count
    }
}

impl<S: ShiperatorCaptain + ShiperatorSailor + Send + Clone>
    rayon::iter::plumbing::UnindexedProducer for Shiperator<S>
{
    type Item = S::Out;

    fn split(mut self) -> (Self, Option<Self>) {
        if !self.shiperator.can_split() {
            return (self, None);
        }
        // Skip proven-empty sources before creating any parallel work for them.
        let current_count = loop {
            let count = self.shiperator.candidate_count(self.start, self.end);
            if count != 0 {
                break count;
            }
            self.start = self.end;
            let Some(end) = self.entities.next_slice() else {
                return (self, None);
            };
            self.start = 0;
            self.end = end;
            self.shiperator.next_slice();
        };
        let max_len = current_count.saturating_add(self.pending_candidate_count());
        if max_len <= self.min_split_len.max(1) {
            return (self, None);
        }

        // Split at a source boundary first. Each child retains the exact source
        // identity, then subdivides its own dense indices by candidate work.
        if !self.entities.follow_up_ptrs.is_empty() {
            let left = Shiperator {
                shiperator: self.shiperator.clone(),
                entities: RawEntityIdAccess::new(self.entities.ptr, Vec::new()),
                is_exact_sized: self.is_exact_sized,
                start: self.start,
                end: self.end,
                min_split_len: self.min_split_len,
            };
            self.end = self.entities.next_slice().unwrap();
            self.start = 0;
            self.shiperator.next_slice();
            return (left, Some(self));
        }

        let new_end = self.shiperator.candidate_midpoint(self.start, self.end);
        let entities = RawEntityIdAccess::new(self.entities.ptr, Vec::new());

        (
            Shiperator {
                shiperator: self.shiperator.clone(),
                entities,
                is_exact_sized: self.is_exact_sized,
                start: self.start,
                end: new_end,
                min_split_len: self.min_split_len,
            },
            Some(Shiperator {
                shiperator: self.shiperator,
                entities: self.entities,
                is_exact_sized: self.is_exact_sized,
                start: new_end,
                end: self.end,
                min_split_len: self.min_split_len,
            }),
        )
    }

    fn fold_with<F>(self, folder: F) -> F
    where
        F: rayon::iter::plumbing::Folder<Self::Item>,
    {
        folder.consume_iter(self)
    }
}

impl<S: ShiperatorCaptain + ShiperatorSailor + Send + Clone> rayon::iter::ParallelIterator
    for ParShiperator<S>
where
    S::Out: Send,
{
    type Item = S::Out;

    #[inline]
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: rayon::iter::plumbing::UnindexedConsumer<Self::Item>,
    {
        let ParShiperator(mut producer, threshold) = self;
        configure_split(&mut producer, threshold);

        rayon::iter::plumbing::bridge_unindexed(producer, consumer)
    }

    #[inline]
    fn opt_len(&self) -> Option<usize> {
        if self.0.is_exact_sized {
            self.0.size_hint().1
        } else {
            None
        }
    }
}

impl<S: ShiperatorCaptain + ShiperatorSailor + Send + Clone> rayon::iter::ParallelIterator
    for WithId<ParShiperator<S>>
where
    S::Out: Send,
{
    type Item = (EntityId, S::Out);

    #[inline]
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: rayon::iter::plumbing::UnindexedConsumer<Self::Item>,
    {
        let WithId(ParShiperator(mut producer, threshold)) = self;
        configure_split(&mut producer, threshold);

        rayon::iter::plumbing::bridge_unindexed(WithId(producer), consumer)
    }
}

impl<S> WithId<ParShiperator<S>> {
    /// Sets the candidate-count split threshold for this iterator.
    ///
    /// See [`ParShiperator::with_split_threshold`] for details. Repeated calls
    /// use the last value, including calls made before `with_id()`.
    ///
    /// # Panics
    ///
    /// Panics if `threshold` is zero.
    pub fn with_split_threshold(self, threshold: usize) -> Self {
        WithId(self.0.with_split_threshold(threshold))
    }
}
