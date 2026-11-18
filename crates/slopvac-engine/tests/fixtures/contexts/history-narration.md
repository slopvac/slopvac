<!-- rule: core:docs-discipline.history-narration -->

<!-- fires: plain body prose -->
# Permission modes

Sessions created without a mode previously started in `default`.

<!-- silent: labelled versioned callout -->
# Permission modes

A session created without a mode starts in the server's default mode.

:::info[Changed in v2.0]

Sessions created without a mode previously started in `default`. Pass an explicit mode if your client depends on that.

:::

<!-- fires: prose after a versioned callout -->
# Permission modes

:::info[Changed in v2.0]

Pass an explicit mode if your client depends on the change.

:::

Sessions created without a mode previously started in `default`.

<!-- fires: unlabelled callout -->
# Permission modes

:::info[Note]

Sessions created without a mode previously started in `default`.

:::

<!-- silent: upgrade section (Stacklok before) -->
# Permission modes

## Upgrade from the posture model

Earlier releases fixed allow-all for the whole process with `--posture`.

- **Sessions created without a mode** start in the server's default mode. Previously, a gRPC session created without a mode started in `default`.

<!-- fires: unversioned earlier-releases text in a body section -->
# Permission modes

## Sessions

Earlier releases fixed allow-all for the whole process. Sessions previously started in `default`.
