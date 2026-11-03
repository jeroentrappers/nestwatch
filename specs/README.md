# Nestwatch specifications

Nestwatch is a private, local-first baby monitor. These documents are the single
source of truth for every Nestwatch implementation. Code is written from these
specs; where code and spec disagree, the spec wins until it is changed.

| Document | Contents |
|----------|----------|
| [product.md](product.md) | Roles, user-facing behaviour, numbered requirements |
| [protocol.md](protocol.md) | Nestwatch Wire Protocol 1 (NWP/1): discovery, pairing, handshake, records, control messages |
| [media.md](media.md) | Audio and video pipelines, codecs, sound activation, jitter handling |
| [security.md](security.md) | Threat model, key management, privacy rules |
| [platforms.md](platforms.md) | Supported platforms, minimum versions, platform obligations |
| [architecture.md](architecture.md) | Rust core, bindings, app structure |
| [test-vectors.md](test-vectors.md) | How conformance vectors are produced and checked |

## Conventions

- The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in
  RFC 2119 / RFC 8174.
- Requirements carry stable identifiers (`FR-PAIR-3`, `NFR-LAT-1`, ...).
  Tests and issues reference them. Identifiers are never reused.
- Integers on the wire are unsigned and big-endian unless stated otherwise.
- `||` denotes byte concatenation.
- Byte strings in text form use base64url without padding (RFC 4648 §5)
  unless stated otherwise.

## Clean-room process

Nestwatch is an independent implementation of a familiar product category. Its
specifications are written from desired behaviour and public standards
(Noise, CBOR, Opus, mDNS, SPAKE2, JPEG, WebSocket). Implementers work from
these documents only and do not consult the source code of other baby
monitor projects. Changes to behaviour go through the specs first.
