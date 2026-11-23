use alloc::vec::Vec;

pub(super) struct SparseSetCloneFunctions<T> {
    pub(super) clone_component: fn(&T) -> T,
    pub(super) clone_storage: fn(&[T]) -> Vec<T>,
}

impl<T: Clone> SparseSetCloneFunctions<T> {
    pub(super) fn new() -> Self {
        SparseSetCloneFunctions {
            clone_component: T::clone,
            clone_storage: |data| data.to_vec(),
        }
    }
}
