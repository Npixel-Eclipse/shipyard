use crate::iter::{Shiperator, ShiperatorCaptain, ShiperatorSailor};

const MIN_SPLIT_LEN: usize = 16;

#[allow(missing_docs)]
pub struct ParShiperator<S>(pub(crate) Shiperator<S>);

impl<S: ShiperatorCaptain + ShiperatorSailor + Send + Clone>
    rayon::iter::plumbing::UnindexedProducer for Shiperator<S>
{
    type Item = S::Out;

    fn split(self) -> (Self, Option<Self>) {
        let follow_up_len = self.entities.follow_up_len();
        let remaining = self.end - self.start;

        let max_len = self.end - self.start + follow_up_len;
        if max_len <= self.min_split_len.max(1) {
            return (self, None);
        }

        let new_end = self.start + (remaining / 2);

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
        let total_len = producer.end - producer.start + producer.entities.follow_up_len();
        let threads = rayon::current_num_threads().max(1);

        producer.min_split_len = (total_len / (threads * 4)).max(MIN_SPLIT_LEN);

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
