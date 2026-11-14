# Cache guide

The cache lives in `~/.cache/slopvac` and holds compiled rule packs. Run `slopvac plugins sync` to refill it.

Offline mode blocks every network call. The flag is `--offline`. Packs that the lockfile pins still load from disk.

The default timeout is 60 seconds. A failed download exits with code 2. Slopvac retries a failed download once.

Maintainers publish packs to GHCR. Verification happens before extraction.

Set `SLOPVAC_CACHE_DIR` to move the cache. Use `--refresh` to force a download.
