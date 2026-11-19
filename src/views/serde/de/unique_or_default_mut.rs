use crate::component::Unique;
use crate::views::UniqueOrDefaultViewMut;
use serde::de::{DeserializeOwned, DeserializeSeed};
use serde::{Deserialize, Deserializer};

/// Builder to customize [`UniqueOrDefaultViewMut`]'s deserialization format.
///
/// Make sure to match the configuration used when serializing.
pub struct UniqueViewOrDefaultMutDeserializer<'tmp, 'view, T: Unique + Default> {
    unique: &'tmp mut UniqueOrDefaultViewMut<'view, T>,
}

impl<'tmp, 'view, T: Unique + Default> UniqueViewOrDefaultMutDeserializer<'tmp, 'view, T> {
    fn new(unique: &'tmp mut UniqueOrDefaultViewMut<'view, T>) -> Self {
        Self { unique }
    }
}

impl<'tmp, 'view, 'de: 'view, T: Unique + Default> DeserializeSeed<'de>
    for UniqueViewOrDefaultMutDeserializer<'tmp, 'view, T>
where
    T: DeserializeOwned,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize_in_place(deserializer, &mut (**self.unique))
    }
}

impl<'view, 'de: 'view, T: Unique + Default> Deserialize<'de> for UniqueOrDefaultViewMut<'view, T>
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
        let unique_view_mut_deserializer = UniqueViewOrDefaultMutDeserializer::new(place);
        DeserializeSeed::deserialize(unique_view_mut_deserializer, deserializer)
    }
}
