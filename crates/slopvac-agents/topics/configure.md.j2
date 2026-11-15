<!-- summary: Settings, config layers and project overrides -->
# Prime: configure

- The project config is `slopvac.toml`. No other file name works.
- It starts with `schema_version = {{ schema_version }}`.
  A file without it is a 2.x config. Slopvac rejects it with exit 2 and the code `config-legacy-version`. Run `slopvac migrate`.
- An unknown key is an error with file, line and column. Keys that start with `x-` are ignored.
- `slopvac lint --explain-config <file>` shows the source file and line of each setting.
- `--config <file>` names one config file.
{% if level != "minimal" %}
## Offline mode

- `--offline`, `SLOPVAC_OFFLINE=1` or `offline = true` under `[network]` turns it on.
- Offline is sticky: no other source turns it off.
- `lint`, `rules`, `explain`, `prime` and `claims` read local files only.
{% endif %}
## Override a topic

- `.slopvac/PRIME.md` replaces topics. A line `<!-- slopvac:topic NAME -->` starts the text for topic NAME.
- A file with no such line replaces the base topic.
- Overrides are plain Markdown. Slopvac renders no template in them.
{% if level == "full" %}
- Guidance detail is `minimal`, `default` or `full`. `slopvac prime --level <level>` sets it for one run.
{% endif %}
