---
description: Rust age crate (str4d/rage) — pure-Rust X25519 recipient and identity parsing with drop zeroization and streaming Encryptor/Decryptor adapters over std::io traits.
---

# rage (`age` Rust Crate) — Streaming Encryption API

## Capabilities and invariants

- The `age` Rust crate (maintained in `str4d/rage`) implements the `age` v1
  file-encryption specification in pure Rust without requiring external CLI
  binaries.
- `age::x25519::Recipient` parses public Bech32 recipients (`age1...`) directly
  via `FromStr`, avoiding hand-rolled Bech32 or key-format validation.
- `age::x25519::Identity` parses private identities (`AGE-SECRET-KEY-1...`) and
  zeroizes key material on drop.
- `age::Encryptor` and `age::Decryptor` wrap standard `std::io::Write` and
  `std::io::Read` streams using authenticated 64 KiB STREAM chunks for bounded
  memory usage on multi-gigabyte payloads.

## Sources

- https://github.com/str4d/rage
- https://docs.rs/age/latest/age/
