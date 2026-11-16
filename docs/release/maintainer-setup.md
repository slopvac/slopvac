# Maintainer setup for candidates

The stable release prerequisites live in [distribution](distribution.md#maintainer-setup): the Homebrew tap and App access, the minisign secret and public key, the release-please App, `RULESET_READ_TOKEN`, the `v*` tag ruleset and immutable releases. Candidates also need the signing key. This page lists only what [release candidates](candidates.md) add. A maintainer does these by hand, because no workflow or script in this repository changes repository, PyPI or GitHub settings.

## PyPI Trusted Publishing for candidates

PyPI matches the publishing workflow by file name and environment. Candidates upload from a new workflow, so register it for the `slopvac` project at <https://pypi.org/manage/project/slopvac/settings/publishing/> before the first candidate:

| Field | Value |
| --- | --- |
| Owner | `slopvac` |
| Repository name | `slopvac` |
| Workflow name | `candidate.yml` |
| Environment name | `pypi` |

The PyPI trusted publisher owner is the `slopvac` GitHub organisation. The release GitHub App is installed on the `slopvac` organisation.

Stable releases keep the registration of `publish-lint.yml` that already exists. If a later change publishes stable wheels from another workflow, register that file the same way.

## The `pypi` environment

In Settings, Environments, `pypi`:

- Add required reviewers (at least one maintainer who did not cut the tag).
- Under deployment branches and tags, allow the `rc/v*` and `v*` tag patterns.
- Add no secrets. Trusted Publishing needs none.

## Protect candidate tags

Create a second ruleset that targets `refs/tags/rc/v*` with the same settings as the `v*` ruleset in [distribution](distribution.md#protect-version-tags): active, with the update, deletion and non-fast-forward rules, and a bypass list that is empty or admin-only.

No workflow checks this ruleset. `scripts/release_ruleset.py` accepts only plain `vX.Y.Z` tags. The stable gate does not rely on it: it needs a passed `candidate.yml` push run for the candidate tag at its current commit, so a moved tag has no passed run until its run passes. The ruleset keeps the tag that testers installed from fixed.

## Check the setup

Run the `Release candidate` workflow by hand (`workflow_dispatch`) on a branch with `dry-run` on. It exercises the build and tests and publishes nothing. The first real upload happens only when someone pushes an `rc/vX.Y.Z-rc.N` tag and approves the `pypi` environment.

## Release signing key

Release artifacts are signed with minisign. The public key is committed at `keys/slopvac-release.pub`.

- Key id: `D9914BDB4570AC6D`
- Public key:

  ```text
  untrusted comment: minisign public key D9914BDB4570AC6D
  RWRtrHBF20uR2Ul9P3Ppdw/e62xuebTxakJxrRL1eyclNXsCQvwoCqzN
  ```

- The secret key lives in the GitHub Actions secret `MINISIGN_SECRET_KEY`. A backup is in the 1Password vault `proj-slopvac`, item `slopvac release minisign key`.

To verify a release file against the committed key:

```sh
minisign -Vm <file> -p keys/slopvac-release.pub
```

The release and candidate workflows sign with this key and check each signature against the committed public key, as [distribution](distribution.md#signatures) describes. The key has no password: create the secret from a key made with `minisign -G -W`. Both workflows stop before they upload when `keys/slopvac-release.pub` or the secret is missing.

### Where the binary uses the key

The `slopvac-plugins` crate compiles `keys/slopvac-release.pub` in with `include_str!`. `KeyPolicy::compiled()` in `sign.rs` makes it the only key for the first-party namespace `slopvac`. `Revocations::with_compiled_root` in `revoke.rs` makes it the root key that verifies the revocation list. Both return an error if the compiled key does not parse, and first-party verification then refuses. Tests inject other keys through the public fields of `KeyPolicy` and through `Revocations::new`, so no test needs the secret key.

### How first-party packs are signed

Release CI signs release artifacts with `MINISIGN_SECRET_KEY` in minisign's default prehashed mode. Signing first-party packs and the revocation list is not wired into a workflow yet, so no first-party pack or revocation list is signed. A signed empty revocation list must be published before the CLI ships. A first-party pack carries a signed statement and its `.minisig` as the two signature layers described in [the threat model](../contract/threat-model.md#53-rotation-and-revocation). The statement names the namespace `slopvac` and the key id `D9914BDB4570AC6D`. The revocation list will be signed with the same key. No one signs by hand and no one reads the secret key.

### Rotate the key

The first-party key set is compiled in, so it changes only with a slopvac release. A publisher-key rotation statement (signed by the old and new key) applies to pinned user keys, not to this key.

1. Generate the new minisign key pair. Store the secret key in the Actions secret `MINISIGN_SECRET_KEY` and a backup in 1Password, without printing it.
2. Replace `keys/slopvac-release.pub` with the new public key, update the key id on this page, and release a binary that embeds it. The embedded-key test in `crates/slopvac-plugins/tests/sign.rs` must change in the same pull request.
3. Re-sign the first-party packs and publish a new revocation list with the new key.
4. If the old key leaked, publish a list that names its key id, signed while the old key is still a root. Binaries that embed the old key keep trusting it until the user upgrades, which is residual risk R8 in the threat model.
