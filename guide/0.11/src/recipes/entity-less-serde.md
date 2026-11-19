# Entity-less serialization

Back in the [serde](../going-further/serde.md) chapter, many possible formats were listed for serializing views.\
By default, views will serialize separately. This has big advantages but serialized size is not one of them.\
In this chapter we'll infer [`EntitiesViewMut`](https://docs.rs/shipyard/0.11/shipyard/struct.EntitiesViewMut.html) content from other views instead of serializing it.

## Serialization

For serialization, we can rely on the default derive format.\
We only have to ignore [`EntitiesViewMut`](https://docs.rs/shipyard/0.11/shipyard/struct.EntitiesViewMut.html) by using `serde(skip)`.

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:ser}}
```

## Deserialization

When serializing structs, `serde` will handle them as maps. Let's start there.\
We'll use `deserialize_in_place` since views don't own their data.

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:de_start}}
```

Now we can add `vm_name` and `vm_favorite_language` deserialization code by replacing the `todo!()` with:

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:struct}}
```

We simply delegate to [`ViewMut`](https://docs.rs/shipyard/0.11/shipyard/struct.ViewMut.html)'s default deserialization for the rest.\
(To be frank there might be a simpler way to go about this, but I don't know it)

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:fields}}
```

We're done deserializing `vm_name` and `vm_favorite_language`. The final touch is to make the entities alive and return.

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:entities}}
```

## Test

Let's make sure everything works as expected.

```rs
{{#include ../../../../tests/book/entity_less_serde.rs:test}}
```
