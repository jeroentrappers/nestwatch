# Conformance and test vectors

## 1. Sources of truth

1. **Public vectors** for building blocks, checked directly:
   Noise (the `cacophony`/`snow` vector set for the three patterns used),
   Argon2id (RFC 9106 §5.3), HKDF-SHA256 (RFC 5869 appendix A),
   ChaCha20-Poly1305 (RFC 8439 §2.8.2), CBOR (RFC 8949 appendix A).
2. **Lull vectors** in `vectors/*.json`, produced by the reference
   implementation from fixed seeds:
   - device IDs from fixed public keys;
   - invite PSK and code PSK derivations;
   - complete handshakes for each mode with fixed static and ephemeral keys,
     including every frame on the wire and the verification number;
   - encoded records of every type and every control message;
   - codec 2 frames for fixed input signals;
   - invitation URIs and their parsed form.

## 2. Independent check

`tools/conformance` re-derives every Lull vector with a separate
implementation that does not share code with the Rust core (Python, using
the `noiseprotocol`, `argon2-cffi` and `cbor2` packages). A vector is
accepted only if both implementations agree. Changes to the protocol must
update the spec, the vectors and both implementations together.

## 3. Behavioural tests

Every requirement in product.md that is testable without hardware has at
least one automated test referencing its identifier, using the sans-I/O
session layer with simulated time, loss and reordering.
