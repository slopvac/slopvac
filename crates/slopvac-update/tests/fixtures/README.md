# Test-only signing fixtures

`test-only.pub` is a throwaway minisign public key. Its secret key was deleted after signing and never touched release CI. It must never appear in `keys/`.

`slopvac-x86_64-unknown-linux-gnu.tar.gz` and its `.minisig` came from `scripts/release_signing.py sign --version 3.1.0` with the real minisign CLI. The signature uses the default prehashed mode and the trusted comment `slopvac-release file:slopvac-x86_64-unknown-linux-gnu.tar.gz version:3.1.0`.

`slopvac-x86_64-unknown-linux-gnu.tar.gz.3.2.0-rc.1.minisig` is the same archive signed with `--version 3.2.0-rc.1`. It stands for a genuinely signed candidate that an attacker replays as a stable update.

The unit tests in `src/verify.rs` use them to show that `ReleaseVerifier` accepts what release CI produces. They also show it rejects a changed archive and a different version.
