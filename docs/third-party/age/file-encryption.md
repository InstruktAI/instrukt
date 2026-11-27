---
description: age v1 file encryption specification — X25519 recipients and identities, authenticated ChaCha20-Poly1305 STREAM 64 KiB chunking, and the distinction between ciphertext integrity and producer authority or freshness.
---

# age — File Encryption Format (v1 Specification)

## Format and cryptographic properties

- `age` (`age-encryption.org/v1`) encrypts streams to one or more public
  recipients (native X25519 Bech32 strings starting with `age1...`) and decrypts
  using the matching private identity (`AGE-SECRET-KEY-1...`).
- Payload encryption uses ChaCha20-Poly1305 in a STREAM construction over
  64 KiB authenticated chunks, enabling constant-memory streaming encryption and
  decryption over multi-gigabyte archives without whole-file buffering.

## Integrity vs authority and freshness

- Successful decryption authenticates both the file header and every payload
  chunk for the supplied private identity.
- Public-key encryption does not establish who produced a ciphertext, when it
  was created, or whether it is the latest version: anyone holding the public
  recipient can encrypt valid ciphertext to it. Producer authority and
  generation freshness must be enforced by the surrounding storage and
  validation protocol.

## Sources

- https://age-encryption.org/v1
- https://github.com/FiloSottile/age
