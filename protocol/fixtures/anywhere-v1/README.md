# Forge Anywhere v1 fixtures

These fixtures are normative. Integers are unsigned big-endian on the wire; `*_hex` values use
lowercase hexadecimal only. Implementations must reproduce `envelope_hex` byte-for-byte and must
reject a wrong encryption key, a changed fixed-header byte, changed ciphertext, a changed
signature, an unsupported version, and any non-canonical trailing bytes.

The private service copies these fixture files into its own repository. It must not import or link
the AGPL Forge protocol crate.

