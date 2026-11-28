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
| `plugins` | The lockfile status and each locked plugin with its state. |
| `trust` | The trust store status, the number of plugin approvals and the pinned keys. |
| `directories` | The cache, data and state directories. |

### 2.1 Update state

The `update.state` value comes from the cache file that the update check writes (policies.md, section 1.2). It is `available` when the cached release is newer than the running version, `current` when it is not, and `unknown` when no readable answer exists or the cached version is not a semantic version. Doctor does not print an answer it cannot compare. Versions compare as semantic versions, so `3.10.0` is newer than `3.9.0`.

`update.checks` is `disabled` when something in section 1.3 of the policies turns the check off. `disabled_by` names the first reason in this order: `offline`, `ci`, `env` (`NO_UPDATE_NOTIFIER`, `SLOPVAC_NO_UPDATE_CHECK` or `DO_NOT_TRACK`) and `config` (`[network] update_check = false`, or a configuration that does not resolve, so no check starts). A disabled check still shows the cached answer.

A newer release does not make `ok` false.

### 2.2 Offline

`offline.sources` lists every source that turned offline mode on: `flag`, `env` and `config`, in that order. Offline is sticky, so one source is enough, and an empty list means online.

### 2.3 Configuration

Doctor resolves the configuration as `lint` does for the working directory: the `--config` file, or the nearest `slopvac.toml` (config.md, section 2), with every file it extends. `config.sources` lists each file in the order the resolution reads it: the `--config` file or the nearest config first, then each extended file, depth first and in `extends` order. A file that more than one file extends is listed once. A config that a plugin supplies (`<plugin>:<name>`) is not listed. A directory or other non-regular file named `slopvac.toml` is not a config, so the search passes over it.

Each entry has a `layer`, a `file` and a `status`. The layer is `explicit` for the `--config` file, `project` for the nearest config, and `extends` for a file that an `extends` entry names. A 2.x config that the search meets (`.slopvac.toml`, or a `[tool.slopvac]` table in `pyproject.toml`) has the layer `project`. The set of layers can grow.

| Status | Meaning |
|---|---|
| `ok` | The file reads, parses and passes validation. |
| `legacy` | A 2.x file. The error says to run `slopvac migrate`. |
| `missing` | The file does not exist. Only `--config` or an `extends` entry can produce it. |
| `unreadable` | The path is not a regular file, is over 1 MiB, is not valid UTF-8, or cannot be read. |
| `invalid` | The file is not valid TOML, has an unsupported `schema_version`, fails validation, or holds an `extends` entry that the resolution refuses. |

Resolution stops at the first failure. The failing file is the last entry, and the files after it are not read or listed. Every entry before it is `ok`, because each file is checked before the files it extends are read. A file that an `extends` entry names and that cannot be read gets the status itself, and the file that names it stays `ok`. An `extends` entry that loops, that passes an `extends` budget (policies.md, section 1.9) or that names a plugin config that is not available makes the file that holds it `invalid`.

The `error` text is one of these fixed messages. It holds no file content. `<line>` is the one-based line of the first problem in the file.

| Case | `error` |
|---|---|
| The file does not exist. | `file not found` |
| The file is over 1 MiB. | `larger than 1048576 bytes` |
| The file is not valid UTF-8. | `not valid UTF-8` |
| Any other read failure. | `cannot read the file` |
| A 2.x file. | ``2.x configuration: run `slopvac migrate` `` |
| Not valid TOML. | `not valid TOML (line <line>)` |
| An unsupported `schema_version`. | `unsupported schema_version` |
| An unknown key. | `unknown key (line <line>)` |
| Any other validation failure. | `invalid configuration (line <line>)` |
| The `extends` entries form a loop. | ``the `extends` entries form a loop (line <line>)`` |
| An `extends` budget is passed. | ``the `extends` chain is over its budget (line <line>)`` |
| An `extends` entry names a plugin config that is not available. | ``an `extends` entry names a plugin config that is not available (line <line>)`` |

A file that an `extends` entry names and that cannot be read for any reason but its size shows `cannot read the file`. `config.ok` is true when every source has status `ok`. A project with no source runs on the defaults, and `config.ok` is true.

### 2.4 Plugins and trust

The lockfile is `.slopvac/plugins.lock` in the directory of the configuration file, or in the working directory if the search finds none. `plugins.lockfile.status` is `absent` when there is no lockfile, which is not a problem. It is `invalid` when the file does not parse, and `unreadable` when the file or the plugin store cannot be read. The `invalid` error holds the location only, such as `invalid value at /plugins/0/digest`, or `not a valid lockfile` for a syntax error. It repeats nothing the file said, because a parser message can echo a rejected value. A pointer segment that is not a lockfile key or an array index shows as `*`.

Each entry in `plugins.items` has the plugin `id`, `version` and `kind` (`data` or `code`), and a `state`: `recorded`, `in-place`, `changed`, `tampered` or `not-installed`. Doctor reports the state without repairing it. `plugins sync` repairs a tampered plugin.

Doctor reads the trust file like every other project file: without blocking, only as a regular file, and under the 8 MiB limit of the store. A missing file is `absent`. A permission error, a non-regular file, an oversize file or a damaged file is `unreadable`. `trust.trusted_plugins` counts the approvals in the trust store. `trust.pinned_keys` lists each pinned namespace with the SHA-256 fingerprint of its key. Doctor prints no stored text as it stands. It parses each stored key and derives the fingerprint itself, so a stored fingerprint that disagrees fails. The namespace must match `^[a-z][a-z0-9-]{0,63}$`. A store with an invalid pin is `unreadable` with the fixed error `the trust store holds an invalid pinned key`, and `pinned_keys` is empty.

`ok` is false when `config.ok` is false, the lockfile status is `invalid` or `unreadable`, any plugin is `tampered`, or the trust store is `unreadable`.

### 2.5 Directories

`directories` has `cache`, `data` and `state`. Each entry gives the `path`, the `source` of the choice (an environment variable name, `default` or `unresolved`), whether it exists, and the total size of the regular files below it. The size walk stops after 10000 entries. It then sets `truncated`, so `bytes` is a lower bound. Links are not followed. The working directory itself shows as `.`. A directory shows `<unresolved>` when the environment has no home to derive it from.

The cache directory is `SLOPVAC_CACHE_DIR` when set and not empty. Otherwise each location follows the XDG base directory specification. An `XDG_*` variable counts only when it holds an absolute path:

| Directory | Location |
|---|---|
| cache | `$XDG_CACHE_HOME/slopvac`, else `~/.cache/slopvac` |
| data | `$XDG_DATA_HOME/slopvac`, else `~/.local/share/slopvac` |
| state | `$XDG_STATE_HOME/slopvac`, else `~/.local/state/slopvac` |

The same locations apply on macOS and under WSL. 3.0 has no native Windows build.

The data directory is the plugin store, and the state directory holds the trust store.

## 3. Redaction

A path in the report is relative to the working directory, or `<external>/<file name>` outside it (policies.md, section 1.7). The separator is `/` on every platform. Messages hold no absolute path and no file content. A per-user directory therefore shows as `<external>/slopvac`, and `source` says how the location was chosen.

## 4. Build status

The directory locations come from the shared resolver in `slopvac-fs`, the same one the plugin store and the trust store use.

## 5. Conformance

The corpus is `schemas/v3/conformance/doctor-report/`. The binary tests in `crates/slopvac/tests/doctor.rs` validate every document they capture against the schema. They cover the rows `cmd:doctor`, `opt:doctor:--format`, `opt:doctor:--binary` and `opt:doctor:--config` of the compatibility matrix.
