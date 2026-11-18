<!-- rule: core:ai-tells-structure.negative-guarantee-core -->

<!-- fires: descriptive guarantee with nothing behind it -->
The release agent never merges a pull request.

<!-- fires: a guarantee in a later paragraph than a requirement -->
You must restart the service.

The importer never deletes a row.

<!-- fires: a guarantee in the next sentence after a requirement -->
You must restart the service. The importer never deletes a row.

<!-- silent: guarantee in a sentence tagged requirement -->
The release agent never merges a pull request, and only CI may tag a release.

<!-- silent: guarantee in a sentence tagged enforced-invariant -->
The release agent never merges a pull request, because branch protection rejects its token.
