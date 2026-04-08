#[cfg(feature = "serde1")]
mod serde;

use core::fmt;
use core::num::NonZeroU64;
use core::str::FromStr;

/// Entity handle.
//
// the id is 64 bits long
// <- 46 index -> <- 16 gen -> <- 2 meta ->
// a generation of !0 is used as a dead entity
//
// inserted and modified component are flagged using metadata
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EntityId(pub(super) NonZeroU64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseEntityIdError;

/// Allows [`EntityId`] to be stored in collections requiring [`Default`], like `TinyVec`.
impl Default for EntityId {
    fn default() -> Self {
        Self::dead()
    }
}

impl EntityId {
    // Number of bits used by the generation
    const GEN_LEN: u64 = 16;
    const INDEX_LEN: u64 = 64 - Self::GEN_LEN;
    const INDEX_MASK: u64 = !(!0 << Self::INDEX_LEN);
    const GEN_MASK: u64 = !(!0 >> Self::GEN_LEN);
    const MAX_GEN: u16 = u16::MAX - 1;

    /// Returns the index part of the `EntityId`.\
    /// ⚠️ You shouldn't use it to index a storage.
    #[inline]
    pub fn index(self) -> u64 {
        (self.0.get() & Self::INDEX_MASK) - 1
    }
    /// Returns the index part of the `EntityId` as an usize.\
    /// ⚠️ You shouldn't use it to index a storage.
    #[inline]
    pub fn uindex(self) -> usize {
        self.index() as usize
    }
    /// Modify the index.
    #[inline]
    pub(crate) fn set_index(&mut self, index: u64) {
        assert!(index < Self::INDEX_MASK);
        // SAFE never zero
        self.0 =
            unsafe { NonZeroU64::new_unchecked((index + 1) | (self.0.get() & !Self::INDEX_MASK)) }
    }
    /// Returns the generation part of the `EntityId`.
    #[inline]
    pub fn gen(self) -> u16 {
        ((self.0.get() & Self::GEN_MASK) >> Self::INDEX_LEN) as u16
    }
    /// Increments the generation, returns Err if gen + 1 == gen::MAX().
    #[inline]
    pub(super) fn bump_gen(&mut self) -> Result<(), ()> {
        if self.gen() < Self::MAX_GEN - 1 {
            // SAFE never zero
            self.0 = unsafe {
                NonZeroU64::new_unchecked(
                    (self.0.get() & !Self::GEN_MASK)
                        | (((self.gen() + 1) as u64) << Self::INDEX_LEN),
                )
            };
            Ok(())
        } else {
            Err(())
        }
    }
    /// Make a new `EntityId` with the given index.
    #[inline]
    pub(crate) fn new(index: u64) -> Self {
        assert!(index < Self::INDEX_MASK);
        // SAFE never zero
        EntityId(unsafe { NonZeroU64::new_unchecked(index + 1) })
    }
    #[inline]
    pub(crate) const fn new_from_parts(index: u64, gen: u16) -> Self {
        assert!(index < Self::INDEX_MASK);

        EntityId(unsafe {
            NonZeroU64::new_unchecked((index + 1) | ((gen as u64) << Self::INDEX_LEN))
        })
    }
    /// Build a new `EntityId` with the given index and generation.
    #[inline]
    pub const fn new_from_index_and_gen(index: u64, gen: u16) -> Self {
        EntityId::new_from_parts(index, gen)
    }
    #[cfg(test)]
    pub(crate) fn zero() -> Self {
        EntityId(NonZeroU64::new(1).unwrap())
    }
    /// Returns a dead `EntityId`, it can be used as a null entity.
    #[inline]
    pub fn dead() -> Self {
        // SAFE not zero
        EntityId(unsafe { NonZeroU64::new_unchecked(!0) })
    }
    #[inline]
    pub(crate) fn bucket(self) -> usize {
        self.uindex() / crate::sparse_set::BUCKET_SIZE
    }
    #[inline]
    pub(crate) fn bucket_index(self) -> usize {
        self.uindex() % crate::sparse_set::BUCKET_SIZE
    }
    #[inline]
    pub(crate) fn max_index() -> u64 {
        Self::INDEX_MASK - 1
    }
    /// Maximum generation of a valid [`EntityId`].\
    /// A dead id will be above that.
    #[inline]
    pub(crate) const fn max_gen() -> u16 {
        Self::MAX_GEN
    }
    #[inline]
    pub(crate) fn is_dead(&self) -> bool {
        (self.0.get() & Self::GEN_MASK) == Self::GEN_MASK
    }
    #[inline]
    pub(crate) fn copy_index(&mut self, other: EntityId) {
        unsafe {
            self.0 = NonZeroU64::new_unchecked(
                (self.0.get() & !Self::INDEX_MASK) | (other.0.get() & Self::INDEX_MASK),
            );
        }
    }
    #[inline]
    pub(crate) fn copy_gen(&mut self, other: EntityId) {
        unsafe {
            self.0 = NonZeroU64::new_unchecked(
                (self.0.get() & !Self::GEN_MASK) | (other.0.get() & Self::GEN_MASK),
            );
        }
    }
    #[inline]
    pub(crate) fn copy_index_gen(&mut self, other: EntityId) {
        unsafe {
            self.0 = NonZeroU64::new_unchecked(self.0.get() | other.0.get());
        }
    }
    /// Returns `EntityId`'s inner representation.
    #[inline]
    pub fn inner(self) -> u64 {
        self.0.get()
    }
    /// Build an `EntityId` from its inner representation.
    #[inline]
    pub fn from_inner(inner: u64) -> Option<EntityId> {
        Some(EntityId(NonZeroU64::new(inner)?))
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == EntityId::dead() {
            f.write_str("dead")
        } else {
            write!(f, "{}.{}", self.index(), self.gen())
        }
    }
}

impl fmt::Debug for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == EntityId::dead() {
            f.write_str("EId(dead)")
        } else {
            write!(f, "EId({}.{})", self.index(), self.gen())
        }
    }
}

impl fmt::Display for ParseEntityIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid entity id")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseEntityIdError {}

impl FromStr for EntityId {
    type Err = ParseEntityIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s == "dead" {
            return Ok(EntityId::dead());
        }

        let (index, gen) = s.split_once('.').ok_or(ParseEntityIdError)?;
        let index = index.parse::<u64>().map_err(|_| ParseEntityIdError)?;
        let gen = gen.parse::<u16>().map_err(|_| ParseEntityIdError)?;

        Ok(EntityId::new_from_index_and_gen(index, gen))
    }
}

impl IntoIterator for EntityId {
    type Item = EntityId;
    type IntoIter = core::iter::Once<EntityId>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        core::iter::once(self)
    }
}

#[test]
fn entity_id() {
    let mut entity_id = EntityId::new(0);
    assert_eq!(entity_id.index(), 0);
    assert_eq!(entity_id.gen(), 0);
    entity_id.set_index(701);
    assert_eq!(entity_id.index(), 701);
    assert_eq!(entity_id.gen(), 0);
    entity_id.bump_gen().unwrap();
    entity_id.bump_gen().unwrap();
    entity_id.bump_gen().unwrap();
    assert_eq!(entity_id.index(), 701);
    assert_eq!(entity_id.gen(), 3);
    entity_id.set_index(554);
    assert_eq!(entity_id.index(), 554);
    assert_eq!(entity_id.gen(), 3);
}

#[test]
fn entity_id_display() {
    let entity_id = EntityId::new_from_index_and_gen(34420, 0);
    assert_eq!(std::format!("{entity_id}"), "34420.0");
    assert_eq!(std::format!("{}", EntityId::dead()), "dead");
}

#[test]
fn entity_id_from_str() {
    assert_eq!(
        "34420.0".parse::<EntityId>(),
        Ok(EntityId::new_from_index_and_gen(34420, 0))
    );
    assert_eq!("dead".parse::<EntityId>(), Ok(EntityId::dead()));
}

#[test]
fn entity_id_round_trips_with_display() {
    let entity_id = EntityId::new_from_index_and_gen(554, 3);
    assert_eq!(std::format!("{entity_id}").parse::<EntityId>(), Ok(entity_id));
}

#[test]
fn entity_id_from_str_rejects_invalid_input() {
    assert_eq!("".parse::<EntityId>(), Err(ParseEntityIdError));
    assert_eq!("34420".parse::<EntityId>(), Err(ParseEntityIdError));
    assert_eq!("EId(34420.0)".parse::<EntityId>(), Err(ParseEntityIdError));
}
