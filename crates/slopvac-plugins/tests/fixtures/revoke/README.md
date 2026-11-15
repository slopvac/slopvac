# Revocation and rotation test vectors

Fixed vectors for `tests/revoke.rs`. Nothing in them comes from the code under test.

- The four keys (`root`, `old`, `new` and an unpublished `other`) come from the `minisign` 0.12
  command line (`minisign -G -W`). Only the public keys are kept.
- `list.json` and `rotation.json` were written by Python
  (`json.dumps(..., sort_keys=True, separators=(",", ":"))`).
- The revoked digest is `hashlib.sha256(b"revoked manifest")`:
  `sha256:9bf2927e4aa4f4343f08fb8878cfd0d4385febc236e2579d78031eff23f004d2`.
- Key ids are bytes 2 to 10 of the base64-decoded key, reversed, as hex:
  `root` is `85B97BE367E4EF44`, `old` is `8384841ACDC75F77`, `new` is `7FF478DC0F88DED4` and
  `other` is `B274295BE578C6F2`. The list revokes `old`.
- `list.minisig` is `minisign -S` by `root` (prehashed, `ED`). `list.legacy.minisig` is
  `minisign -S -l` by `root` (legacy, `Ed`), which the verifier must refuse.
  `list.forged.minisig` is `minisign -S` by `other` over the same bytes.
- `rotation.old.minisig` and `rotation.new.minisig` are `minisign -S` by `old` and `new` over
  `rotation.json`.
- `minisign -V` verifies every signature except the forged one with the matching public key.
