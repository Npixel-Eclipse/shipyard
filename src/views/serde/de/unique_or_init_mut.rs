use crate::component::Unique;
use crate::views::UniqueOrInitViewMut;
use serde::de::{DeserializeOwned, DeserializeSeed};
use serde::{Deserialize, Deserializer};

/// Builder to customize [`UniqueOrInitViewMut`]'s deserialization format.
///
/// Make sure to match the configuration used when serializing.
pub struct UniqueOrInitViewMutDeserializer<'tmp, 'view, T: Unique> {
    unique: &'tmp mut UniqueOrInitViewMut<'view, T>,
}

impl<'tmp, 'view, T: Unique + Send + Sync> UniqueOrInitViewMutDeserializer<'tmp, 'view, T> {
    fn new(unique: &'tmp mut UniqueOrInitViewMut<'view, T>) -> Self {
        Self { unique }
    }
}

impl<'tmp, 'view, 'de: 'view, T: Unique + Send + Sync> DeserializeSeed<'de>
    for UniqueOrInitViewMutDeserializer<'tmp, 'view, T>
where
    T: DeserializeOwned,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut maybe_unique: Option<T> = None;
        Deserialize::deserialize_in_place(deserializer, &mut maybe_unique)?;

        if let (Some(unique), Some(storage)) = (maybe_unique, self.unique.get_mut()) {
            **storage = unique;
        }

        Ok(())
    }
}

impl<'view, 'de: 'view, T: Unique + Send + Sync> Deserialize<'de> for UniqueOrInitViewMut<'view, T>
where
    T: DeserializeOwned,
{
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        panic!("UniqueViewMut cannot be directly deserialized. Use deserialize_in_place instead.")
    }

    fn deserialize_in_place<D>(deserializer: D, place: &mut Self) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let unique_view_mut_deserializer = UniqueOrInitViewMutDeserializer::new(place);
        DeserializeSeed::deserialize(unique_view_mut_deserializer, deserializer)
    }
}
