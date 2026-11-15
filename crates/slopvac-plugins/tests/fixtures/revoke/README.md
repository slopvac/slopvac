# Revocation and rotation test vectors

Fixed vectors for `tests/revoke.rs`. Nothing in them comes from the code under test.

- The four keys (`root`, `old`, `new` and an unpublished `other`) come from the `minisign` 0.12
  command line (`minisign -G -W`). Only the public keys are kept.
- `list.json` and `rotation.json` were written by Python
  (`json.dumps(..., sort_keys=True, separators=(",", ":"))`).
- The revoked digest is `hashlib.sha256(b"revoked manifest")`:
  `sha256:9bf2927e4aa4f4343f08fb8878cfd0d4385febc236e2579d78031eff23f004d2`.
- Key ids are bytes 2 to 10 of the base64-decoded key, reversed, as hex:
  `root` is `BF7AC81F9F42CA98`, `old` is `C0E5E326B2F9B470`, `new` is `A7628DAD436BB320` and
  `other` is `28F53B61D3486CD8`. The list revokes `old`.
- The list is issued `2026-11-12T00:00:00Z` and expires `2026-12-12T00:00:00Z`.
- `rotation.json` binds the whole new key: `newKey` is the base64 line of `new.pub`.
- `list.minisig` is `minisign -S` by `root` (prehashed, `ED`). `list.legacy.minisig` is
  `minisign -S -l` by `root` (legacy, `Ed`), which the verifier must refuse.
  `list.forged.minisig` is `minisign -S` by `other` over the same bytes.
- `rotation.old.minisig` and `rotation.new.minisig` are `minisign -S` by `old` and `new` over
  `rotation.json`.
- `minisign -V` verifies every signature except the forged one with the matching public key.
- `equivocation.a.json` and `equivocation.b.json` are two lists with the same `issued` and
  `expires`. `a` revokes the digest above and `b` revokes nothing. A separate key
  (`equivocation.root.pub`, also from `minisign -G -W`) signs both with `minisign -S`, so both
  are authentic, and `minisign -V` verifies both.
