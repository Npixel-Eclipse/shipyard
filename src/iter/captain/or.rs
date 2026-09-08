use crate::iter::ShiperatorCaptain;
use crate::or::OrWindow;

impl<T: ShiperatorCaptain, U: ShiperatorCaptain> ShiperatorCaptain for OrWindow<(T, U)> {
    #[inline]
    fn has_stable_membership(&self) -> bool {
        self.storages.0.has_stable_membership() && self.storages.1.has_stable_membership()
    }

    #[inline]
    fn can_split(&self) -> bool {
        // A right-source worker probes left membership to suppress duplicates.
        // Mutable modification tracking writes those per-slot timestamps, so
        // keep that query on one producer instead of racing with the probe.
        self.storages.0.has_stable_membership()
            && self.storages.0.can_split()
            && self.storages.1.can_split()
    }
    #[inline]
    unsafe fn get_captain_data(&self, _index: usize) -> Self::Out {
        unreachable!()
    }

    #[inline]
    fn next_slice(&mut self) {
        self.set_slice(self.current_slice + 1);
    }

    #[inline]
    fn set_slice(&mut self, slice: usize) {
        self.current_slice = slice;
        if slice < self.left_slices {
            self.storages.0.set_slice(slice);
        } else {
            self.storages.1.set_slice(slice - self.left_slices);
        }
    }

    #[inline]
    fn slice_index(&self) -> usize {
        self.current_slice
    }

    #[inline]
    fn sail_time(&self) -> usize {
        self.storages
            .0
            .sail_time()
            .saturating_add(self.storages.1.sail_time())
    }

    #[inline]
    fn is_exact_sized(&self) -> bool {
        false
    }

    #[inline]
    fn unpick(&mut self) {
        (self.storages).0.unpick();
        (self.storages).1.unpick();
    }

    #[inline]
    fn has_no_candidates(&self) -> bool {
        self.storages.0.has_no_candidates() && self.storages.1.has_no_candidates()
    }

    #[inline]
    fn next_possible(&self, index: usize) -> usize {
        if self.current_slice < self.left_slices {
            self.storages.0.next_possible(index)
        } else {
            self.storages.1.next_possible(index)
        }
    }

    #[inline]
    fn next_possible_in(&self, index: usize, end: usize) -> usize {
        if self.current_slice < self.left_slices {
            self.storages.0.next_possible_in(index, end)
        } else {
            self.storages.1.next_possible_in(index, end)
        }
    }

    #[inline]
    fn previous_possible(&self, end: usize) -> usize {
        if self.current_slice < self.left_slices {
            self.storages.0.previous_possible(end)
        } else {
            self.storages.1.previous_possible(end)
        }
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_count(&self, start: usize, end: usize) -> usize {
        self.candidate_count_at(self.current_slice, start, end)
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_count_at(&self, slice: usize, start: usize, end: usize) -> usize {
        if slice < self.left_slices {
            self.storages.0.candidate_count_at(slice, start, end)
        } else {
            self.storages
                .1
                .candidate_count_at(slice - self.left_slices, start, end)
        }
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_midpoint(&self, start: usize, end: usize) -> usize {
        if self.current_slice < self.left_slices {
            self.storages.0.candidate_midpoint(start, end)
        } else {
            self.storages.1.candidate_midpoint(start, end)
        }
    }
}
