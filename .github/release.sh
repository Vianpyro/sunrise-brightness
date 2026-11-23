#!/usr/bin/env bash
# Bumps the version when the commits since the last tag call for it, then tags the version of
# Cargo.toml if it has no tag yet and starts release.yml on that tag. Run by version.yml from the
# repository root, with full history and tags. DRY_RUN=1 prints the commands instead.
# Adapted from github.com/Vianpyro/pals (.github/release.sh).
#
# Bump rule (Conventional Commits): a breaking change raises the major; feat raises the minor; fix,
# perf and build(deps) the patch (a dependency update reaches users only through a release, since
# the crates are compiled into the binary); anything else releases nothing. A version raised by
# hand is kept.
set -euo pipefail

run() { if [ -n "${DRY_RUN:-}" ]; then echo "+ $*"; else "$@"; fi; }

name=$(sed -n 's/^name = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || { echo "no version line in Cargo.toml (CRLF line endings?)" >&2; exit 1; }
last=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)

if [ -n "$last" ]; then
  log=$(git log --format=%B "$last..HEAD")
  IFS=. read -r major minor patch <<< "${last#v}"
  scope='(\([^()]+\))?'
  if grep -qE "^[a-z]+$scope!:|^BREAKING[ -]CHANGE:" <<< "$log"; then
    next=$((major + 1)).0.0
  elif grep -qE "^feat$scope:" <<< "$log"; then
    next=$major.$((minor + 1)).0
  elif grep -qE "^(fix$scope|perf$scope|build\(deps\)):" <<< "$log"; then
    next=$major.$minor.$((patch + 1))
  else
    next=${last#v}
  fi

  if [ "$(printf '%s\n' "$version" "$next" | sort -V | tail -n 1)" != "$version" ]; then
    run sed -i "0,/^version = \"$version\"$/s//version = \"$next\"/" Cargo.toml
    # The package's own entry in Cargo.lock, and nothing else: no index, no toolchain needed
    run sed -i "/^name = \"$name\"$/{n;s/^version = \".*\"$/version = \"$next\"/}" Cargo.lock
    run git -c user.name='github-actions[bot]' \
      -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
      commit -m "chore(release): v$next" -- Cargo.toml Cargo.lock
    run git push origin HEAD:main
    version=$next
  fi
fi

if git rev-parse -q --verify "refs/tags/v$version" > /dev/null; then
  echo "v$version is already released"
  exit 0
fi
run git tag "v$version"
run git push origin "v$version"
# A tag pushed with GITHUB_TOKEN triggers no workflow, but a dispatch does: release.yml builds and
# publishes the release of that tag.
run gh workflow run release.yml --ref "v$version"
