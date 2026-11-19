use crate::component::Unique;
use crate::views::UniqueViewMut;
use serde::de::{DeserializeOwned, DeserializeSeed};
use serde::{Deserialize, Deserializer};

/// Builder to customize [`UniqueViewMut`]'s deserialization format.
///
/// Make sure to match the configuration used when serializing.
pub struct UniqueViewMutDeserializer<'tmp, 'view, T: Unique> {
    unique: &'tmp mut UniqueViewMut<'view, T>,
}

impl<'tmp, 'view, T: Unique> UniqueViewMutDeserializer<'tmp, 'view, T> {
    fn new(unique: &'tmp mut UniqueViewMut<'view, T>) -> Self {
        Self { unique }
    }
}

impl<'tmp, 'view, 'de: 'view, T: Unique> DeserializeSeed<'de>
    for UniqueViewMutDeserializer<'tmp, 'view, T>
where
    T: DeserializeOwned,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize_in_place(deserializer, &mut self.unique.unique.value)
    }
}

impl<'view, 'de: 'view, T: Unique> Deserialize<'de> for UniqueViewMut<'view, T>
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
        let unique_view_mut_deserializer = UniqueViewMutDeserializer::new(place);
        DeserializeSeed::deserialize(unique_view_mut_deserializer, deserializer)
    }
}
