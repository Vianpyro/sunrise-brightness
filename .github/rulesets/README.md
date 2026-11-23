# Rulesets

The repository's rulesets, kept here so they are reviewed like the rest. GitHub does not read them
from this directory: apply a change with

```bash
gh api -X POST repos/Vianpyro/sunrise-brightness/rulesets --input .github/rulesets/main.json
gh api -X POST repos/Vianpyro/sunrise-brightness/rulesets --input .github/rulesets/tags.json
```

(`-X PUT .../rulesets/<id>` to update one that exists; `gh api repos/Vianpyro/sunrise-brightness/rulesets` lists them.)

- `main.json`: main is never deleted nor force-pushed. No required check: maintainers push to main
  directly, and so does the release bot (`.github/release.sh`). Dependabot's pull requests wait for
  a green CI anyway (`dependabot-merge.yml`). To require pull requests, require the `gate` check of
  `ci.yml` (the only one needed) and add the GitHub Actions app as a bypass actor, so the bump
  commit still goes through.
- `tags.json`: a release tag is never moved nor deleted once pushed.
