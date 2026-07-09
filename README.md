# flatrecord

[![Rust CI](https://github.com/apfitzge/flatrecord/actions/workflows/ci.yml/badge.svg)](https://github.com/apfitzge/flatrecord/actions/workflows/ci.yml)

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
- Fixed-width primitives, `bool`, `[T; N]`, `String`, and `Vec<T>` for primitive
  `T` are supported. Structs are single records; enums with one-field tuple
  variants become tagged unions (the variant index is the 2-byte wire tag).

## Example

See [`examples/records.rs`](examples/records.rs) for a runnable end-to-end example
(`cargo run --example records`) covering the derive, encoding, and dynamic decoding
against an exported schema.

The `Schema` itself is serializable (via [`wincode`]), so a producer can ship the
schema bytes alongside the record stream and the consumer can reconstruct it,
prepare it once, and decode many records.

[`wincode`]: https://crates.io/crates/wincode

## License

Dual-licensed under either the [MIT license](https://opensource.org/licenses/MIT)
or the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0),
at your option.
