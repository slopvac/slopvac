# Maintainer setup for candidates

The stable release prerequisites live in [distribution](distribution.md#maintainer-setup): the Homebrew tap and App access, the minisign secret and public key, the release-please App with Administration: Read-only, the `v*` tag ruleset and immutable releases. Candidates also need the signing key. This page lists only what [release candidates](candidates.md) add. A maintainer does these by hand, because no workflow or script in this repository changes repository, PyPI or GitHub settings.

## PyPI Trusted Publishing

PyPI matches the publishing workflow by file name and environment. Candidates and stable 3.x releases both upload from `publish-pypi.yml`, so register it for the `slopvac` project at <https://pypi.org/manage/project/slopvac/settings/publishing/> before the first candidate:

| Field | Value |
| --- | --- |
| Owner | `slopvac` |
| Repository name | `slopvac` |
| Workflow name | `publish-pypi.yml` |
| Environment name | `pypi` |

The PyPI trusted publisher owner is the `slopvac` GitHub organisation. The release GitHub App is installed on the `slopvac` organisation.

The 2.x registration of `publish-lint.yml` stays while 2.x releases remain. No other workflow uploads to PyPI.

## The `pypi` environment

In Settings, Environments, `pypi`:

- Add required reviewers (at least one maintainer who did not cut the tag).
- Under deployment branches and tags, allow the `rc/v*` and `v*` tag patterns.
- Add no secrets. Trusted Publishing needs none.

## The `candidate-signing` environment

The `sign` job of `publish-pypi.yml` runs in the `candidate-signing` environment and is the only candidate job that reads `MINISIGN_SECRET_KEY`. The job runs only for a publishing run: a push of an `rc/v*` tag, or a manual run on that tag with `dry-run` off. A dry run never signs. In Settings, Environments, `candidate-signing`:

- Under deployment branches and tags, allow only the `rc/v*` tag pattern. Allow no branch and no `v*` tag.
- Add the `MINISIGN_SECRET_KEY` environment secret.
- Add required reviewers only if you want a second approval before signing. The `pypi` environment already gates the upload.

The environment is separate from `release` so that adding `rc/v*` never lets a candidate tag reach the environment that signs stable releases.

## Protect candidate tags

Create a second ruleset that targets `refs/tags/rc/v*` with the same settings as the `v*` ruleset in [distribution](distribution.md#protect-version-tags): active, with the update, deletion and non-fast-forward rules, and a bypass list that is empty or admin-only.

No workflow checks this ruleset. `scripts/release_ruleset.py` accepts only plain `vX.Y.Z` tags. The stable gate does not rely on it: it needs a passed `publish-pypi.yml` push run for the candidate tag at its current commit, so a moved tag has no passed run until its run passes. The ruleset keeps the tag that testers installed from fixed.

## Pin the tag rulesets

`scripts/release_ruleset.py check` trusts a tag ruleset only while its ID and `updated_at` match `scripts/release_ruleset_pins.json`. After you create or edit a tag ruleset, review it, then run this as a repository admin and commit the result:

```sh
python3 scripts/release_ruleset.py update-pins --repo slopvac/slopvac
```

`update-pins` is the only writer of the pin file. It reads the rulesets with your token and refuses to write unless it can see each ruleset's `bypass_actors` and they are empty or admin-only. A token without admin rights therefore cannot update the pins. Review the pin file change like a change to the ruleset itself.

## Check the setup

Run the `Publish to PyPI` workflow by hand (`workflow_dispatch`) on a branch with `dry-run` on. It exercises the build and tests and publishes nothing. The first real upload happens only when someone pushes an `rc/vX.Y.Z-rc.N` or `vX.Y.Z` tag and approves the `pypi` environment.

## Release signing key

Release artifacts are signed with minisign. The public key is committed at `keys/slopvac-release.pub`.

- Key id: `D9914BDB4570AC6D`
- Public key:

  ```text
  untrusted comment: minisign public key D9914BDB4570AC6D
  RWRtrHBF20uR2Ul9P3Ppdw/e62xuebTxakJxrRL1eyclNXsCQvwoCqzN
  ```

- The secret key lives in the GitHub Actions secret `MINISIGN_SECRET_KEY`, stored on the `release`, `candidate-signing` and `revocations` environments. A repository-level copy remains only until the first candidate signs in `candidate-signing`. Then delete it with `gh secret delete MINISIGN_SECRET_KEY -R slopvac/slopvac`, and the environments are its only home. A backup is in the 1Password vault `proj-slopvac`, item `slopvac release minisign key`.

To verify a release file against the committed key:

```sh
minisign -Vm <file> -p keys/slopvac-release.pub
```

The release and candidate workflows sign with this key and check each signature against the committed public key, as [distribution](distribution.md#signatures) describes. The key has no password: create the secret from a key made with `minisign -G -W`. Both workflows stop before they upload when `keys/slopvac-release.pub` or the secret is missing.

### Where the binary uses the key

The `slopvac-plugins` crate compiles `keys/slopvac-release.pub` in with `include_str!`. `KeyPolicy::compiled()` in `sign.rs` makes it the only key for the first-party namespace `slopvac`. `Revocations::with_compiled_root` in `revoke.rs` makes it the root key that verifies the revocation list. Both return an error if the compiled key does not parse, and first-party verification then refuses.

The contract assigns first-party plugin verification to a separate compiled plugins key and reserves the release key for binaries and the revocation list ([plugins contract, section 10](../contract/plugins.md), [threat model, section 5.2](../contract/threat-model.md)). The plugins key is not issued yet: `plugins_key()` in `sign.rs` returns no key. There is a known implementation gap: `KeyPolicy::compiled()` falls back to the release key for the first-party namespace, although the contract requires that namespace to fail closed without a usable compiled plugins key.

The `slopvac-update` crate includes the same file in `verify.rs`, plus the optional `NEXT_RELEASE_KEY_FILE` constant in `verify.rs`. `ReleaseVerifier` checks the `.minisig` of the archive that `slopvac self update` downloads against both keys. It accepts prehashed signatures only. It refuses a missing, tampered, legacy or foreign-key signature before it replaces the binary. It also requires the signed trusted comment to name the requested archive and version, so an older or foreign-target archive that the key signed does not install. `scripts/release_signing.py` writes that comment.

Tests inject other keys through the public fields of `KeyPolicy`, through `Revocations::new` and through `Runtime::verifier`, so no test needs the secret key.

### How first-party plugins are signed

Release CI signs release artifacts with `MINISIGN_SECRET_KEY` in minisign's default prehashed mode. The `Revocation list` workflow signs the revocation list the same way, as [the last section](#the-signed-revocation-list) describes. Signing first-party plugins is not wired into a workflow yet, so no first-party plugin is signed. A first-party plugin carries a signed statement and its `.minisig` as the two signature layers described in [the threat model](../contract/threat-model.md#53-rotation-and-revocation). The statement names the namespace `slopvac` and the key id `D9914BDB4570AC6D`. The revocation list uses the same key. No one signs by hand and no one reads the secret key.

### Rotate the key

The trusted release keys are compiled in, so they change only with a slopvac release. `slopvac self update` trusts a list of up to two keys: the current key in `keys/slopvac-release.pub` and an optional next key in the `NEXT_RELEASE_KEY_FILE` constant in `crates/slopvac-update/src/verify.rs`. A signature from either key verifies, and the trusted comment must still match. The constant is `None` when no rotation is under way.

A binary trusts only the keys it was built with. If a rotation skipped the next-key step, a client could never verify a release signed with the new key, and it would need a manual reinstall. The next key therefore ships in a release before anything is signed with it.

A publisher-key rotation statement (signed by the old and new key) applies to pinned user keys, not to this key.

1. Generate the next minisign key pair with `minisign -G -W`. Keep the secret key out of the repository and the transcript. Store a backup in 1Password. Do not touch the `MINISIGN_SECRET_KEY` environment secrets yet.
2. Set `NEXT_RELEASE_KEY_FILE` to the text of the new public key, in a pull request that updates the key ids on this page and in [distribution](distribution.md#signatures).
3. Release. This rotation release is signed with the current key, still in the `MINISIGN_SECRET_KEY` secret of each of the three signing environments, and embeds both public keys. Clients that update to it now trust the next key.
4. Wait until enough clients have updated to the rotation release. Clients that skip it cannot verify a release signed with the next key.
5. Promote the next key in one pull request: move the key text from `NEXT_RELEASE_KEY_FILE` into `keys/slopvac-release.pub` and set the constant back to `None`. After it merges and before the release workflow runs, replace `MINISIGN_SECRET_KEY` with the new secret in each of the three signing environments. The release workflow checks each signature against `keys/slopvac-release.pub`, so the secret and that file must change together. The release is signed with the new key. Clients on the rotation release accept it through their next key, and it embeds only the new key.
6. Publish a new revocation list with the new key.

Under the two-key contract, a release-key rotation changes trust for binaries and the revocation list, not plugins. The compiled plugins key verifies first-party plugins and the first-party plugin index. Replacing that compiled key requires a slopvac release. Until the implementation gap above is fixed, promoting a new release key also changes the key used to verify the `slopvac` namespace. No first-party plugin is signed yet, so none needs re-signing in this procedure.

#### After a key compromise

If the signing key leaked, rotation does not repair existing installs. Anyone with the old secret can sign an update that a client with the old key accepts, so a client that updates through `self update` cannot recover. Users must reinstall from a source they verify themselves: a release checked against the key id published in this repository, or a package manager. Publish the new key id and the reinstall instruction, then publish a revocation list that names the old key id, signed while the old key is still a root. Binaries that embed the old key keep trusting it until the user reinstalls, which is residual risk R8 in the threat model.

## The signed revocation list

Every online `slopvac plugins sync` fetches a signed revocation list from `ghcr.io/slopvac/slopvac-revocations:latest`. The list names publisher keys and artifact digests that must not run. The list this workflow publishes is empty, because nothing is revoked yet. Its signature must verify with `keys/slopvac-release.pub`.

A list is valid for 30 days. The `Revocation list` workflow (`revocations.yml`) signs a new one every Monday and pushes it, so one is always in force. If the weekly runs stop and the list lapses, every online sync fails with exit 2 until someone pushes a new list (K6 in [the threat model](../contract/threat-model.md)). Offline runs keep using the cached list and print a notice.

The workflow has two jobs:

- `build-sign` runs in the `revocations` environment with `contents: read`. It is the only job that reads `MINISIGN_SECRET_KEY`. It runs `scripts/revocation_list.py build`, which writes the canonical list, signs it in prehashed mode and checks the result against the committed public key.
- `push` has `packages: write` and no secret. It checks the signature again, pushes the two files with a pinned `oras` and then pulls the result back to check it.

The `revocations` environment is separate from `release` on purpose. This scheduled job only ever needs `main`, so the environment deploys from `main` alone, with no admin bypass. The `release` environment allows `main` and `v*` tags, and `candidate-signing` allows `rc/v*` tags only. With a separate environment, a tag can never reach the key through this workflow. The job also has an `if` guard for `main`, as an extra layer, and so does `push`. The workflow has no `pull_request` trigger. Before `push` replaces the list, it resolves the published manifest and pulls the list. It refuses to continue unless the new `issued` time is strictly later, so a retried older run cannot roll clients back. Only a clear "manifest not found" counts as nothing published, and a scheduled run treats even that as a failure. Any other error, such as an authentication or network error, a server error or a layer that cannot be fetched, also fails the run.

### First run

1. Run the `Revocation list` workflow by hand on `main` (`workflow_dispatch`) with `first_publication` set to true, and approve the `revocations` environment if it asks. This is the only kind of run that may find nothing published, and the only one that may replace a published list that fails verification (see below). It fails if the published list verifies. Leave the option off for every later manual run.
2. Open the new `slopvac-revocations` package under the `slopvac` organisation's packages. Packages in an organisation start private. Change its visibility to public, and link it to the `slopvac/slopvac` repository so the workflow keeps write access. While it is private, users cannot pull the list anonymously and every online sync fails.
3. Check the result without a login: `oras manifest fetch ghcr.io/slopvac/slopvac-revocations:latest` must show the artifact type `application/vnd.slopvac.revocations.v1` and two layers, the list and then its signature.

Rerun the workflow, without `first_publication`, whenever the list has lapsed. A rerun issues a newer list, which replaces the old one. The CLI refuses a list with an older `issued` time than the one it cached.

### Recovery and key rotation

The candidate list is always signed with the current key, `keys/slopvac-release.pub`. The published list must verify before a run replaces it. It may verify with the current key or with a key kept in `keys/` as `slopvac-release-<KEYID>.pub` (the previous or next key of a rotation, see [Rotate the key](#rotate-the-key)). Delete that file to stop trusting the key. Clients verify only against the key compiled into the binary, so a list from another key cannot be used and the next run replaces it.

If the published list fails verification (a bad signature, an unlisted key, or a malformed or non-canonical list), a normal run stops and leaves it in place. To replace it, run the workflow by hand on `main` with `first_publication` set to true. That run logs a `::warning::` that names the reason, and then replaces the list. The same option refuses to replace a list that verifies.
