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

The `slopvac-plugins` crate compiles `keys/slopvac-release.pub` in with `include_str!`. `KeyPolicy::compiled()` in `sign.rs` makes it the only key for the first-party namespace `slopvac`. `Revocations::with_compiled_root` in `revoke.rs` makes it the root key that verifies the revocation list. Both return an error if the compiled key does not parse, and first-party verification then refuses.

The `slopvac-update` crate includes the same file in `verify.rs`. `ReleaseVerifier` checks the `.minisig` of the archive that `slopvac self update` downloads. It accepts prehashed signatures only. It refuses a missing, tampered, legacy or foreign-key signature before it replaces the binary. It also requires the signed trusted comment to name the requested archive and version, so an older or foreign-target archive that the key signed does not install. `scripts/release_signing.py` writes that comment.

Tests inject other keys through the public fields of `KeyPolicy`, through `Revocations::new` and through `Runtime::verifier`, so no test needs the secret key.

### How first-party packs are signed

Release CI signs release artifacts with `MINISIGN_SECRET_KEY` in minisign's default prehashed mode. The `Revocation list` workflow signs the revocation list the same way, as [the last section](#the-signed-revocation-list) describes. Signing first-party packs is not wired into a workflow yet, so no first-party pack is signed. A first-party pack carries a signed statement and its `.minisig` as the two signature layers described in [the threat model](../contract/threat-model.md#53-rotation-and-revocation). The statement names the namespace `slopvac` and the key id `D9914BDB4570AC6D`. The revocation list uses the same key. No one signs by hand and no one reads the secret key.

### Rotate the key

The first-party key set is compiled in, so it changes only with a slopvac release. A publisher-key rotation statement (signed by the old and new key) applies to pinned user keys, not to this key.

1. Generate the new minisign key pair. Store the secret key in the Actions secret `MINISIGN_SECRET_KEY` and a backup in 1Password, without printing it.
2. Replace `keys/slopvac-release.pub` with the new public key, update the key id on this page, and release a binary that embeds it. The embedded-key tests in `crates/slopvac-plugins/tests/sign.rs` and `crates/slopvac-update/src/verify.rs` must change in the same pull request.
3. Re-sign the first-party packs and publish a new revocation list with the new key.
4. If the old key leaked, publish a list that names its key id, signed while the old key is still a root. Binaries that embed the old key keep trusting it until the user upgrades, which is residual risk R8 in the threat model.

## The signed revocation list

Every online `slopvac plugins sync` fetches a signed revocation list from `ghcr.io/slopvac/slopvac-revocations:latest`. The list names publisher keys and artifact digests that must not run. The list this workflow publishes is empty, because nothing is revoked yet. Its signature must verify with `keys/slopvac-release.pub`.

A list is valid for 30 days. The `Revocation list` workflow (`revocations.yml`) signs a new one every Monday and pushes it, so one is always in force. If the weekly runs stop and the list lapses, every online sync fails with exit 2 until someone pushes a new list (K6 in [the threat model](../contract/threat-model.md)). Offline runs keep using the cached list and print a notice.

The workflow has two jobs:

- `build-sign` runs in the `revocations` environment with `contents: read`. It is the only job that reads `MINISIGN_SECRET_KEY`. It runs `scripts/revocation_list.py build`, which writes the canonical list, signs it in prehashed mode and checks the result against the committed public key.
- `push` has `packages: write` and no secret. It checks the signature again, pushes the two files with a pinned `oras` and then pulls the result back to check it.

The `revocations` environment is separate from `release` on purpose. This scheduled job only ever needs `main`, so the environment deploys from `main` alone, with no admin bypass. The `release` environment also allows `v*` tags. With a separate environment, a tag can never reach the key through this workflow. The job also has an `if` guard for `main`, as an extra layer, and so does `push`. The workflow has no `pull_request` trigger. Before `push` replaces the list, it resolves the published manifest and pulls the list. It refuses to continue unless the new `issued` time is strictly later, so a retried older run cannot roll clients back. Only a clear "manifest not found" counts as nothing published, and a scheduled run treats even that as a failure. Any other error, such as an authentication or network error, a server error or a layer that cannot be fetched, also fails the run.

### First run

1. Run the `Revocation list` workflow by hand on `main` (`workflow_dispatch`) with `first_publication` set to true, and approve the `revocations` environment if it asks. This is the only run that may find nothing published. It fails if a list already exists. Leave the option off for every later manual run.
2. Open the new `slopvac-revocations` package under the `slopvac` organisation's packages. Packages in an organisation start private. Change its visibility to public, and link it to the `slopvac/slopvac` repository so the workflow keeps write access. While it is private, users cannot pull the list anonymously and every online sync fails.
3. Check the result without a login: `oras manifest fetch ghcr.io/slopvac/slopvac-revocations:latest` must show the artifact type `application/vnd.slopvac.revocations.v1` and two layers, the list and then its signature.

Rerun the workflow, without `first_publication`, whenever the list has lapsed. A rerun issues a newer list, which replaces the old one. The CLI refuses a list with an older `issued` time than the one it cached.
