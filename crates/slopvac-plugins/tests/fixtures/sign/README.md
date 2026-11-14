# Signature test vector

A fixed vector for `tests/sign.rs`. Nothing in it comes from the code under test.

- `minisign.pub` and the two signatures come from the `minisign` 0.12 command line
  (`minisign -G -W`, `minisign -S`, and `minisign -S -l` for the legacy format).
  `minisign -V` verifies both signatures with `minisign.pub`.
- `statement.json` was written by Python
  (`json.dumps(..., sort_keys=True, separators=(",", ":"))`), with layer digests from
  `hashlib.sha256` over `{}` (config) and `rules` (the one layer).
- Key id `684BF07F758E0DD7` is bytes 2 to 10 of the base64-decoded key, reversed, as hex.
- Fingerprint `331d813cac2254b59314d744d5a1f12aad3f7c39598d80f8466fc64655d41e11` is
  `hashlib.sha256` over the 42 raw key bytes.
- `statement.minisig` signs the prehashed format (`ED`). `statement.legacy.minisig`
  signs the same bytes in the legacy format (`Ed`), which the verifier must refuse.
