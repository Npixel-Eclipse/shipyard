use crate::entity_id::EntityId;
use crate::iter::{Shiperator, ShiperatorCaptain, ShiperatorSailor, WithId};
use crate::sparse_set::RawEntityIdAccess;
use alloc::vec::Vec;

const MIN_SPLIT_LEN: usize = 16;

#[allow(missing_docs)]
pub struct ParShiperator<S>(pub(crate) Shiperator<S>);

impl<S> ParShiperator<S> {
    /// Returns the [`EntityId`] alongside the component(s).
    pub fn with_id(self) -> WithId<Self> {
        WithId(self)
    }
}

fn configure_split<S: ShiperatorCaptain>(producer: &mut Shiperator<S>) {
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
        let mut producer = self.0;
        configure_split(&mut producer);

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
        let WithId(ParShiperator(mut producer)) = self;
        configure_split(&mut producer);

        rayon::iter::plumbing::bridge_unindexed(WithId(producer), consumer)
    }
}
