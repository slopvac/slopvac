# Slopvac 3.0 doctor

Status: contract for 3.0. Bead `slopvac-8l4.12.5`. Network policy: `policies.md`, sections 1.2, 1.3 and 1.7. The JSON shape is [`doctor-report`](../../schemas/v3/doctor-report.schema.json), and the schema holds the field descriptions.

`slopvac doctor` reports the state of the binary, its configuration and its local data. It answers one question: what does a run on this machine see? It reads local files only, opens no connection, and changes nothing on disk.

## 1. Command line

```text
slopvac doctor [--format text|json] [--config FILE] [--offline]
```

- `--format` defaults to `text`. Both formats print to standard output.
- `--config FILE` names one configuration file and skips discovery, as it does for `lint`. A relative path is relative to the working directory.
- `--binary` is gone. It named the Vale executable, and 3.0 has no Vale. The option is a usage error (exit 2).

The exit code is 0 when `ok` is true and 2 when it is false. Doctor prints the report in both cases, so a script reads the cause from the document. Exit 1 never occurs.

## 2. Report

The 2.x report had its own `schema_version` of 1. The 3.0 report uses 3, like every v3 root. The Python and Vale sections are gone. Sections:

| Key | Content |
|---|---|
| `slopvac` | Version, install method and the command that upgrades it. |
| `platform` | Operating system and architecture. |
| `update` | Whether update checks run, why not, and the cached answer. |
| `offline` | Whether offline mode is on, and which sources turned it on. |
| `config` | Each configuration source with a status. |
| `packs` | The lockfile status and each locked pack with its state. |
| `trust` | The trust store status, the number of plugin approvals and the pinned keys. |
| `directories` | The cache, data and state directories. |

### 2.1 Update state

The `update.state` value comes from the cache file that the update check writes (policies.md, section 1.2). It is `available` when the cached release is newer than the running version, `current` when it is not, and `unknown` when no readable answer exists or the cached version is not a semantic version. Doctor does not print an answer it cannot compare. Versions compare as semantic versions, so `3.10.0` is newer than `3.9.0`.

`update.checks` is `disabled` when something in section 1.3 of the policies turns the check off. `disabled_by` names the first reason in this order: `offline`, `ci`, `env` (`NO_UPDATE_NOTIFIER`, `SLOPVAC_NO_UPDATE_CHECK` or `DO_NOT_TRACK`) and `config` (`[network] update_check = false`). A disabled check still shows the cached answer.

A newer release does not make `ok` false.

### 2.2 Offline

`offline.sources` lists every source that turned offline mode on: `flag`, `env` and `config`, in that order. Offline is sticky, so one source is enough, and an empty list means online.

### 2.3 Configuration

Each entry has a `layer` (`explicit` for `--config`, `project` for the nearest `slopvac.toml`), a `file` and a `status`:

| Status | Meaning |
|---|---|
| `ok` | The file parses and declares `schema_version = 3`. |
| `legacy` | A 2.x file. The error says to run `slopvac migrate`. |
| `missing` | The file does not exist. Only `--config` can produce it. |
| `unreadable` | The path is not a regular file, is over 1 MiB, or cannot be read. |
| `invalid` | The file is not valid TOML or has an unsupported `schema_version`. |

The `error` text is a fixed message with no file content. `config.ok` is true when every source has status `ok`. A project with no source runs on the defaults, and `config.ok` is true.

### 2.4 Packs and trust

The lockfile is `.slopvac/packs.lock` in the directory of the configuration file, or in the working directory if the search finds none. `packs.lockfile.status` is `absent` when there is no lockfile, which is not a problem. It is `invalid` when the file does not parse, and `unreadable` when the file or the pack store cannot be read. The `invalid` error holds the location only, such as `invalid value at /packs/0/digest`, or `not a valid lockfile` for a syntax error. It repeats nothing the file said, because a parser message can echo a rejected value. A pointer segment that is not a lockfile key or an array index shows as `*`.

Each entry in `packs.items` has the pack `id`, `version` and `kind`, and a `state`: `recorded`, `in-place`, `changed`, `tampered` or `not-installed`. Doctor reports the state without repairing it. `plugins sync` repairs a tampered pack.

Doctor reads the trust file like every other project file: without blocking, only as a regular file, and under the 8 MiB limit of the store. A missing file is `absent`. A permission error, a non-regular file, an oversize file or a damaged file is `unreadable`. `trust.trusted_plugins` counts the approvals in the trust store. `trust.pinned_keys` lists each pinned namespace with the SHA-256 fingerprint of its key. Doctor prints fingerprints only.

`ok` is false when `config.ok` is false, the lockfile status is `invalid` or `unreadable`, any pack is `tampered`, or the trust store is `unreadable`.

### 2.5 Directories

`directories` has `cache`, `data` and `state`. Each entry gives the `path`, the `source` of the choice (an environment variable name, `default` or `unresolved`), whether it exists, and the total size of the regular files below it. The size walk stops after 10000 entries. It then sets `truncated`, so `bytes` is a lower bound. Links are not followed. The working directory itself shows as `.`. A directory shows `<unresolved>` when the environment has no home to derive it from.

The cache directory is `SLOPVAC_CACHE_DIR` when set. Otherwise all three follow the XDG layout on Unix (`XDG_CACHE_HOME`, `XDG_DATA_HOME` and `XDG_STATE_HOME`, falling back to `~/.cache`, `~/.local/share` and `~/.local/state`, each with a `slopvac` subdirectory) and `%LOCALAPPDATA%\slopvac` on Windows. The data directory is the pack store, and the state directory holds the trust store.

## 3. Redaction

A path in the report is relative to the working directory, or `<external>/<file name>` outside it (policies.md, section 1.7). Messages hold no absolute path and no file content. Directory entries therefore show `<external>/slopvac` for a per-user directory, and `source` says how the location was chosen.

## 4. Build status

Three inputs come from local stand-ins until the beads that own them land:

- The install method is always `unknown`, and doctor starts no update refresh. `slopvac-8l4.12.22` swaps in `slopvac-update`.
- Configuration discovery handles one `slopvac.toml` and ignores `extends`. `slopvac-8l4.12.23` swaps in the config layer.
- The directory locations come from the environment. `slopvac-8l4.12.24` swaps in the shared resolver.

## 5. Conformance

The corpus is `schemas/v3/conformance/doctor-report/`. The binary tests in `crates/slopvac/tests/doctor.rs` validate every document they capture against the schema. They cover the rows `cmd:doctor`, `opt:doctor:--format`, `opt:doctor:--binary` and `opt:doctor:--config` of the compatibility matrix.
