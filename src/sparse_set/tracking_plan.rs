//! Query-local upper bounds from the existing 64-slot tracking timestamps.
//! A candidate slot still needs its normal per-component timestamp/join checks.

use super::TRACKING_CHUNK_SIZE;
use alloc::{boxed::Box, vec::Vec};

const MIN_DENSE_FALLBACK_CHUNKS: usize = 32;

pub(crate) enum TrackingPlan {
    Empty,
    Dense { len: usize },
    Sparse(SparsePlan),
}

pub(crate) struct SparsePlan {
    // Retained for candidate-work splitting when parallel iteration is enabled.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    ranges: Box<[CandidateRange]>,
    chunk_bits: Box<[usize]>,
    len: usize,
    slots: usize,
    chunks: usize,
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
struct CandidateRange {
    start: usize,
    end: usize,
    /// Number of candidate slots in all preceding ranges.
    #[cfg(any(feature = "parallel", test))]
    prefix: usize,
}

impl TrackingPlan {
    /// `candidate` must return true for missing/uncertain chunk metadata.
    pub(crate) fn build(len: usize, mut candidate: impl FnMut(usize) -> bool) -> Self {
        let chunk_count = len.div_ceil(TRACKING_CHUNK_SIZE);
        if chunk_count == 0 {
            return Self::Empty;
        }
        if chunk_count == 1 {
            return if candidate(0) {
                Self::Dense { len }
            } else {
                Self::Empty
            };
        }
        let bits = usize::BITS as usize;
        let mut chunk_bits = Vec::new();
        let mut chunks = 0;
        for chunk in 0..chunk_count {
            if candidate(chunk) {
                if chunk_bits.is_empty() {
                    chunk_bits.resize(chunk_count.div_ceil(bits), 0usize);
                }
                chunk_bits[chunk / bits] |= 1usize << (chunk % bits);
                chunks += 1;
            }
        }
        if chunks == 0 {
            return Self::Empty;
        }
        // A broad upper bound is preferable to building many ranges for dense
        // work. This fallback still uses the ordinary exact tracking checks.
        if chunks == chunk_count
            || (chunk_count >= MIN_DENSE_FALLBACK_CHUNKS && chunks >= chunk_count.div_ceil(2))
        {
            return Self::Dense { len };
        }

        let mut ranges = Vec::new();
        let mut run_start = None;
        let mut run_end = 0;
        let mut slots = 0;
        // Build ranges from the captured bitmap, without re-reading timestamps.
        for (word_index, &word) in chunk_bits.iter().enumerate() {
            let mut remaining = word;
            while remaining != 0 {
                let chunk = word_index * bits + remaining.trailing_zeros() as usize;
                remaining &= remaining - 1;
                let start = chunk * TRACKING_CHUNK_SIZE;
                if start != run_end {
                    if let Some(start_of_run) = run_start.take() {
                        ranges.push(CandidateRange {
                            start: start_of_run,
                            end: run_end,
                            #[cfg(any(feature = "parallel", test))]
                            prefix: slots,
                        });
                        slots += run_end - start_of_run;
                    }
                }
                run_start.get_or_insert(start);
                run_end = (start + TRACKING_CHUNK_SIZE).min(len);
            }
        }
        if let Some(start) = run_start {
            ranges.push(CandidateRange {
                start,
                end: run_end,
                #[cfg(any(feature = "parallel", test))]
                prefix: slots,
            });
            slots += run_end - start;
        }
        Self::Sparse(SparsePlan {
            ranges: ranges.into_boxed_slice(),
            chunk_bits: chunk_bits.into_boxed_slice(),
            len,
            slots,
            chunks,
        })
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Relative scan work, not an exact item count or elapsed-time estimate.
    #[inline]
    pub(crate) fn scan_cost(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Dense { len } => len.saturating_add(len.div_ceil(TRACKING_CHUNK_SIZE)),
            Self::Sparse(plan) => plan.slots.saturating_add(plan.chunks),
        }
    }

    #[inline]
    pub(crate) fn next(&self, index: usize) -> usize {
        match self {
            Self::Empty => usize::MAX,
            Self::Dense { .. } => index,
            Self::Sparse(plan) => {
                if index >= plan.len {
                    return plan.len;
                }
                let bits = usize::BITS as usize;
                let chunk = index / TRACKING_CHUNK_SIZE;
                let mut word = chunk / bits;
                let mut candidates = plan.chunk_bits[word] & (usize::MAX << (chunk % bits));
                while candidates == 0 {
                    word += 1;
                    if word == plan.chunk_bits.len() {
                        return plan.len;
                    }
                    candidates = plan.chunk_bits[word];
                }
                let next_chunk = word * bits + candidates.trailing_zeros() as usize;
                index.max(next_chunk * TRACKING_CHUNK_SIZE)
            }
        }
    }

    #[cfg(any(feature = "parallel", test))]
    #[inline]
    fn rank(&self, index: usize) -> usize {
        match self {
            Self::Empty => 0,
            Self::Dense { len, .. } => index.min(*len),
            Self::Sparse(plan) => {
                let pos = plan.ranges.partition_point(|range| range.end <= index);
                plan.ranges.get(pos).map_or(plan.slots, |range| {
                    range.prefix + index.saturating_sub(range.start)
                })
            }
        }
    }

    #[cfg(any(feature = "parallel", test))]
    #[inline]
    pub(crate) fn count(&self, start: usize, end: usize) -> usize {
        self.rank(end) - self.rank(start)
    }

    #[cfg(any(feature = "parallel", test))]
    #[inline]
    pub(crate) fn midpoint(&self, start: usize, end: usize) -> usize {
        let rank = self.rank(start) + self.count(start, end) / 2;
        match self {
            Self::Sparse(plan) => {
                let pos = plan
                    .ranges
                    .partition_point(|range| range.prefix + range.end - range.start <= rank);
                let range = &plan.ranges[pos];
                range.start + rank - range.prefix
            }
            _ => start + (end - start) / 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_and_split_match_candidate_slots_for_every_subrange() {
        let len = 257;
        for mask in 0..32 {
            let plan = TrackingPlan::build(len, |chunk| mask & (1 << chunk) != 0);
            let candidate: Vec<_> = (0..len).filter(|i| mask & (1 << (i / 64)) != 0).collect();
            let mut actual = Vec::new();
            let mut index = 0;
            while index < len {
                index = plan.next(index);
                if index < len {
                    actual.push(index);
                    index += 1;
                }
            }
            assert_eq!(actual, candidate);
            for start in [0, 1, 63, 64, 65, 127, 128, 192, 256] {
                for end in [64, 127, 128, 191, 192, 256, 257] {
                    if end <= start {
                        continue;
                    }
                    let expected = candidate.iter().filter(|&&i| start <= i && i < end).count();
                    assert_eq!(plan.count(start, end), expected);
                    if expected > 1 {
                        let split = plan.midpoint(start, end);
                        assert!(start < split && split < end);
                        assert_eq!(plan.count(start, split), expected / 2);
                        assert_eq!(plan.count(split, end), expected - expected / 2);
                    }
                }
            }
        }
    }

    #[test]
    fn dense_and_empty_plans_do_not_allocate_ranges() {
        assert!(matches!(
            TrackingPlan::build(0, |_| true),
            TrackingPlan::Empty
        ));
        assert!(matches!(
            TrackingPlan::build(4096, |_| false),
            TrackingPlan::Empty
        ));
        assert!(matches!(
            TrackingPlan::build(4097, |_| true),
            TrackingPlan::Dense { .. }
        ));
    }
}
