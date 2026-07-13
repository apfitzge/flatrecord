# flatrecord

[![Rust CI](https://github.com/anza-xyz/flatrecord/actions/workflows/ci.yml/badge.svg)](https://github.com/anza-xyz/flatrecord/actions/workflows/ci.yml)

Compact, schema-described binary record encoding with a derive macro.

`flatrecord` encodes structs and enums to a tightly packed little-endian wire
format via `#[derive(FlatRecord)]`. Each type can also export a self-describing
`Schema`, so a consumer that never saw the original Rust types can decode the
bytes reflectively.

The wire format has no alignment padding, so scalar fields are decoded with
unaligned reads rather than borrowed in place. Variable-length and byte fields
(`String`, `Vec<u8>`, `[u8; N]`) are returned as slices borrowed directly from
the payload.

- **Typed path** — encode/decode directly to and from your own types.
- **Dynamic path** — decode against a runtime `Schema` with `PreparedSchema` +
  `DynamicRecord`, walking fields by name and reading their values.
- Fixed-width primitives, `bool`, `[T; N]`, `String`, `Vec<T>` for primitive
  `T`, and simple `FlatEnum` fields are supported. Structs are single records;
  enums with one-field tuple variants become tagged unions (the variant index is
  the 2-byte wire tag).

## Example

See [`examples/struct_root.rs`](examples/struct_root.rs) for a runnable end-to-end
struct-root example (`cargo run --example struct_root`) with a `FlatEnum` `state`
field and dynamic decoding against an exported schema. [`examples/struct_root_vec.rs`](examples/struct_root_vec.rs)
shows a struct root with `Vec<u64>`, and [`examples/records.rs`](examples/records.rs)
demonstrates tagged-union roots.

The `Schema` itself is serializable (via [`wincode`]), so a producer can ship the
schema bytes alongside the record stream and the consumer can reconstruct it,
prepare it once, and decode many records.

[`wincode`]: https://crates.io/crates/wincode

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
