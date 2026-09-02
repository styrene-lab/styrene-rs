# Operation-Scoped Authorization

## Intent

Provide one reusable Styrene authorization policy for operation, resource, and
context decisions so consumers do not maintain lossy local operation-to-role
tables or trusted-principal parsing.

## Scope

This change covers principals, exact and prefix operation grants, explicit deny
precedence, role bundles, resource and context constraints, trusted issuer input,
structured decisions, audit fields, effective-policy discovery, and test helpers.

It does not define consumer HTTP routes, UI navigation, authentication protocols,
or service-specific operation catalogs.

## Success criteria

- Consumers can authorize their own operation strings without a local role table.
- Unauthenticated or untrusted proxy identity never becomes a principal.
- Decisions are structured, deterministic, explainable, and safe to audit.
- Existing coarse Styrene roles remain available through explicit grant bundles.
- Effective-policy discovery uses the same evaluator as enforcement.
