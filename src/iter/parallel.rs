use crate::entity_id::EntityId;
use crate::iter::{Shiperator, ShiperatorCaptain, ShiperatorSailor, WithId};

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
        + producer.entities.follow_up_len();
    let threads = rayon::current_num_threads().max(1);

    producer.min_split_len = (total_len / (threads * 4)).max(MIN_SPLIT_LEN);
}

impl<S: ShiperatorCaptain + ShiperatorSailor + Send + Clone>
    rayon::iter::plumbing::UnindexedProducer for Shiperator<S>
{
    type Item = S::Out;

    fn split(self) -> (Self, Option<Self>) {
        let follow_up_len = self.entities.follow_up_len();
        let remaining = self.end - self.start;

        let max_len = self.shiperator.candidate_count(self.start, self.end) + follow_up_len;
        if max_len <= self.min_split_len.max(1) {
            return (self, None);
        }

        let new_end = if follow_up_len == 0 {
            self.shiperator.candidate_midpoint(self.start, self.end)
        } else {
            self.start + remaining / 2
        };

        let (entities, other_entities) = self.entities.split_at(follow_up_len / 2);

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
                entities: other_entities,
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
