# Cache guide

The cache lives in `~/.cache/slopvac` and holds compiled rule packs. Entries expire after 24 hours. Run `slopvac plugins sync` to refill it.

Offline mode blocks every network call. The flag is `--offline`. Packs that the lockfile pins still load from disk.

The default timeout is 30 seconds. A failed download exits with code 2. Slopvac never retries a failed download.

Maintainers publish packs to GHCR. Each pack carries a minisign signature. Verification happens before extraction.

Set `SLOPVAC_CACHE_DIR` to move the cache. The directory must be writable.
