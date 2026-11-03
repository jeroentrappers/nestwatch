# Open questions

Implementers record here every place where the specs are ambiguous,
incomplete or contradictory, with the interpretation they chose. The spec
owner resolves each entry by changing the specs and removing it here.

| ID | Spec section | Question | Interim choice |
|----|--------------|----------|----------------|
| CRYPTO-1 | protocol.md §5.3 | The listener checks the station key against `device_id` "after the handshake". May it check earlier? | The listener checks as soon as the station's static key is known (after reading message 2) and aborts before writing message 3, so a station with the wrong key never receives the PSK-bound message it could guess against offline. On the wire this is identical for honest stations. |
| CRYPTO-2 | protocol.md §5.3 | Which text form of `station_device_id_ascii` goes into the code salt? | The canonical form: 16 lower-case base32 characters without hyphens (as in the `id` TXT key). |
| CRYPTO-3 | protocol.md §5.3 | Argon2id version and optional inputs are not stated. | Version 0x13 (RFC 9106), password = the 10 ASCII characters of the normalised code, no secret key, no associated data. |
| CRYPTO-4 | protocol.md §5.2 | In `IKpsk2` the PSK is only mixed into message 2, so the station completes the handshake without proof that the listener knew the PSK. When has the invite handshake "succeeded"? | The station adds the listener to its trusted list only after the listener's first transport message (its `hello`) decrypts. |
| CRYPTO-5 | protocol.md §5.6 | "Close before the nonce reaches 2^32": is nonce 2^32 − 1 still usable? | Nonces 0 .. 2^32 − 1 are usable (2^32 messages per direction); encrypting or decrypting with nonce 2^32 fails with a nonce-exhausted error. |
| CRYPTO-6 | protocol.md §2.1, security.md §5 | The stored format of the device secret is not specified. | 32 raw X25519 secret bytes as generated (not pre-clamped); the public key is X25519(secret, 9), clamping applied as RFC 7748 specifies. |
| CRYPTO-7 | protocol.md §5.3 | Code normalisation: which whitespace is removed, and how is non-ASCII input upper-cased? | Only ASCII spaces and hyphens are removed; upper-casing is ASCII-only; any other character (tabs, `U`, non-ASCII) rejects the input. |
| CRYPTO-8 | protocol.md §2.2 | Should device ID parsing accept the grouped and upper-case forms? | Parsing ignores hyphens and accepts either case; the canonical (displayed in TXT) form is lower case without hyphens. |
