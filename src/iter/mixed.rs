use crate::entity_id::EntityId;
use crate::iter::{
    captain::ShiperatorCaptain, into_shiperator::strip_plus, output::ShiperatorOutput,
    sailor::ShiperatorSailor,
};

const NON_CAPTAIN_FACTOR: f32 = 0.5;

/// Iterator over multiple storages.
pub struct Mixed<S> {
    pub(crate) shiperator: S,
    pub(crate) mask: u32,
}

unsafe impl<S: Send> Send for Mixed<S> {}

impl<S: Clone> Clone for Mixed<S> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            shiperator: self.shiperator.clone(),
            mask: self.mask,
        }
    }
}

impl<S: ShiperatorOutput> ShiperatorOutput for Mixed<S> {
    type Out = S::Out;
}

macro_rules! impl_shiperator_output {
    ($(($type: ident, $index: tt))+) => {
        impl<$($type: ShiperatorOutput),+> ShiperatorOutput for Mixed<($($type,)+)> {
            type Out = ($($type::Out,)+);
        }

        impl<$($type: ShiperatorCaptain),+> ShiperatorCaptain for Mixed<($($type,)+)> {
            #[inline]
            fn has_stable_membership(&self) -> bool {
                $(self.shiperator.$index.has_stable_membership())&&+
            }

            #[inline]
            fn can_split(&self) -> bool {
                $(self.shiperator.$index.can_split())&&+
            }
            #[inline]
            unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
                ($(
                    self.shiperator.$index.get_captain_data(index),
                )+)
            }

            #[inline]
            fn next_slice(&mut self) {
                $(
                    if self.mask & (1 << $index) != 0 {
                        self.shiperator.$index.next_slice();
                    }
                )+
            }

            #[inline]
            fn set_slice(&mut self, slice: usize) {
                $(if self.mask & (1 << $index) != 0 { self.shiperator.$index.set_slice(slice); })+
            }

            #[inline]
            fn slice_index(&self) -> usize {
                $(if self.mask & (1 << $index) != 0 { return self.shiperator.$index.slice_index(); })+
                0
            }

            #[inline]
            #[allow(clippy::cast_precision_loss)]
            fn sail_time(&self) -> usize {
                strip_plus!($(+{
                    let sail_time = self.shiperator.$index.sail_time() as f32;

                    if self.mask & (1 << $index) != 0 {
                        sail_time as usize
                    } else {
                        (sail_time * NON_CAPTAIN_FACTOR) as usize
                    }
                }
                )+)
            }

            #[inline]
            fn is_exact_sized(&self) -> bool {
                // True if mask flags all iterated storages
                self.mask.count_ones() == strip_plus!($(+{let _: $type; 1})+)
            }

            #[inline]
            fn unpick(&mut self) {
                self.mask = 0;

                $(
                    self.shiperator.$index.unpick();
                )+
            }

            #[inline]
            fn has_no_candidates(&self) -> bool {
                $(if self.shiperator.$index.has_no_candidates() { return true; })+
                false
            }
            #[cfg(feature = "parallel")]
            #[inline]
            fn candidate_count(&self, start: usize, end: usize) -> usize {
                $(if self.mask & (1 << $index) != 0 { return self.shiperator.$index.candidate_count(start, end); })+
                end - start
            }
            #[cfg(feature = "parallel")]
            #[inline]
            fn candidate_count_at(&self, slice: usize, start: usize, end: usize) -> usize {
                $(if self.mask & (1 << $index) != 0 { return self.shiperator.$index.candidate_count_at(slice, start, end); })+
                end - start
            }
            #[cfg(feature = "parallel")]
            #[inline]
            fn candidate_midpoint(&self, start: usize, end: usize) -> usize {
                $(if self.mask & (1 << $index) != 0 { return self.shiperator.$index.candidate_midpoint(start, end); })+
                start + (end - start) / 2
            }
            #[inline]
            fn next_possible(&self, index: usize) -> usize {
                $(
                    if self.mask & (1 << $index) != 0 {
                        return self.shiperator.$index.next_possible(index);
                    }
                )+

                index
            }
            #[inline]
            fn previous_possible(&self, end: usize) -> usize {
                $(if self.mask & (1 << $index) != 0 { return self.shiperator.$index.previous_possible(end); })+
                end
            }
        }

        impl<$($type: ShiperatorSailor),+> ShiperatorSailor for Mixed<($($type,)+)> {
            type Index = ($($type::Index,)+);

            #[inline]
            unsafe fn get_sailor_data(&self, index: Self::Index) -> Self::Out {
                ($(
                    self.shiperator.$index.get_sailor_data(index.$index),
                )+)
            }

            #[inline]
            fn indices_of(&self, eid: EntityId, index: usize, ) -> Option<Self::Index> {
                // A membership probe may come from another OR source. Only
                // captain_indices_of is allowed to reuse this dense index.
                Some(($(self.shiperator.$index.indices_of(eid, index)?,)+))
            }

            #[inline]
            fn captain_indices_of(&self, eid: EntityId, index: usize) -> Option<Self::Index> {
                Some(($(
                    if self.mask & (1 << $index) != 0 {
                        if let Some(index) = self.shiperator.$index.captain_indices_of(eid, index) {
                            index
                        } else {
                            return None
                        }
                    } else {
                        if let Some(index) = self.shiperator.$index.indices_of(eid, index) {
                            index
                        } else {
                            return None
                        }
                    },
                )+))
            }

            #[inline]
            fn index_from_usize(index: usize) -> Self::Index {
                ($(
                    $type::index_from_usize(index),
                )+)
            }
        }
    };
}

macro_rules! shiperator_output {
    ($(($type: ident, $index: tt))+; ($type1: ident, $index1: tt) $(($queue_type: ident, $queue_index: tt))*) => {
        impl_shiperator_output![$(($type, $index))*];
        shiperator_output![$(($type, $index))* ($type1, $index1); $(($queue_type, $queue_index))*];
    };
    ($(($type: ident, $index: tt))+;) => {
        impl_shiperator_output![$(($type, $index))*];
    }
}

#[cfg(not(feature = "extended_tuple"))]
shiperator_output![(A, 0); (B, 1) (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)];
#[cfg(feature = "extended_tuple")]
shiperator_output![
    (A, 0); (B, 1) (C, 2) (D, 3) (E, 4) (F, 5) (G, 6) (H, 7) (I, 8) (J, 9)
    (K, 10) (L, 11) (M, 12) (N, 13) (O, 14) (P, 15) (Q, 16) (R, 17) (S, 18) (T, 19)
    (U, 20) (V, 21) (W, 22) (X, 23) (Y, 24) (Z, 25) (AA, 26) (BB, 27) (CC, 28) (DD, 29)
    (EE, 30) (FF, 31)
];
