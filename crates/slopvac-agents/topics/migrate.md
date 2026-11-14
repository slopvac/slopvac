<!-- summary: Move a 2.x project to 3.0 -->
# Prime: migrate

Slopvac 3.0 has no 2.x compatibility mode and no deprecation window.

1. Run `slopvac migrate` to preview the changes. It compares findings before and after.
2. Review the preview with the user, then apply it.
3. Run `slopvac lint <paths> --format json` and compare with the earlier result.

- A 2.x config exits 2 with `config-legacy-version`. Nothing is linted until you migrate.
- A removed flag exits 2: `--profile`, `--rules-dir`, `--comments` (use `--mode code-comments`) and `--no-vale`.
- Rule ids gain a pack namespace. `slopvac migrate` rewrites directive ids and steering blocks.
- A 2.x setting with no 3.0 equivalent gives `config-legacy-dropped`. Decide it with the user.
{% if level == "full" %}
- Lint JSON has `schema_version` {{ schema_version }}. `version` is now `slopvac_version`, and `notes` is now `notices`.
- Branch on notice and error codes, never on message text.
{% endif %}
