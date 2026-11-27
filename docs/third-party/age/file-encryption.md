---
description: age v1 file encryption format and Rust age crate — X25519 recipients and identities, authenticated ChaCha20-Poly1305 STREAM 64 KiB chunking, and the distinction between ciphertext integrity and producer authority or freshness.
---

# age — File Encryption (v1 Specification & Rust Crate)

## Format and cryptographic properties

- `age` (`age-encryption.org/v1`) encrypts streams to one or more public
  recipients (native X25519 Bech32 strings starting with `age1...`) and decrypts
  using the matching private identity (`AGE-SECRET-KEY-1...`).
- Payload encryption uses ChaCha20-Poly1305 in a STREAM construction over
  64 KiB authenticated chunks, allowing constant-memory streaming encryption and
  decryption over multi-gigabyte archives without whole-file buffering.
- The Rust `age` crate provides pure-Rust recipient parsing
  (`age::x25519::Recipient`), identity parsing (`age::x25519::Identity`,
  zeroized on drop), and streaming `Encryptor` / `Decryptor` adapters over
  standard `std::io::Write` and `std::io::Read`.

## Integrity vs authority and freshness

- Successful decryption authenticates both the file header and every payload
  chunk for the supplied private identity.
- Public-key encryption does not establish who produced a ciphertext, when it
  was created, or whether it is the latest generation: anyone holding the
  public recipient can encrypt valid ciphertext to it. Producer authority and
  generation freshness must be enforced by the surrounding storage and
  validation protocol.

## Sources

- https://age-encryption.org/v1
- https://github.com/FiloSottile/age
- https://docs.rs/age/latest/age/
