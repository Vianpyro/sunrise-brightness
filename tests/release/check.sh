#!/usr/bin/env bash
# Checks the commit-msg hook and .github/release.sh: the bump rule in dry run, then one real run
# against a throwaway remote, gh replaced by a stub. Run by the lint job of ci.yml.
# Adapted from github.com/Vianpyro/pals (tests/release/check.sh).
set -euo pipefail

root=$(pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

hook() { printf '%s\n' "$1" > "$tmp/message"; sh "$root/.githooks/commit-msg" "$tmp/message" 2> /dev/null; }
accepts() { hook "$1" || { echo "hook refused: $1" >&2; exit 1; }; }
refuses() { if hook "$1"; then echo "hook accepted: $1" >&2; exit 1; fi; }

accepts 'feat(ui): add a dark theme'
accepts 'fix: SIGTERM, SIGINT and SIGHUP no longer crash the app'
accepts 'fix!: drop the legacy config format'
accepts 'build(deps): bump the crates group with 5 updates (#33)'
accepts 'ci(deps): bump actions/checkout from 7.0.0 to 7.0.1'
accepts "Merge branch 'main' of github.com:Vianpyro/sunrise-brightness"
accepts 'Revert "feat: add a dark theme"'
refuses 'Updated README'
refuses 'feat: add a dark theme.'
refuses 'feature: add a dark theme'
refuses 'feat:add a dark theme'
refuses 'Bump the dependencies group across 1 directory with 5 updates'

# A repository like this one: Cargo.toml, and a Cargo.lock where another crate has the same version
repo() {
  local tag=$1 version=$2
  rm -rf "$tmp/repo" "$tmp/remote.git"
  git init -q --bare "$tmp/remote.git"
  git init -q -b main "$tmp/repo"
  cd "$tmp/repo"
  git config user.name test
  git config user.email test@example.com
  git config core.autocrlf false
  git remote add origin "$tmp/remote.git"
  printf '[package]\nname = "app"\nversion = "%s"\n\n[dependencies]\npolling = "3"\n' "${tag#v}" > Cargo.toml
  printf '[[package]]\nname = "app"\nversion = "%s"\n\n[[package]]\nname = "polling"\nversion = "%s"\n' "${tag#v}" "${tag#v}" > Cargo.lock
  git add Cargo.toml Cargo.lock
  git commit -qm 'feat: init'
  git tag "$tag"
  sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml
  sed -i "/^name = \"app\"$/{n;s/^version = \".*\"$/version = \"$version\"/}" Cargo.lock
  git commit -qam 'chore: set the version' --allow-empty
}

# Repository at tag $1 (version of Cargo.toml: $2), plus one commit per message; prints the tag
# release.sh would create, or the version already released.
releases() {
  local tag=$1 version=$2
  shift 2
  repo "$tag" "$version"
  for message in "$@"; do git commit -qm "$message" --allow-empty; done
  DRY_RUN=1 bash "$root/.github/release.sh" | sed -n 's/^+ git tag \(v[^ ]*\)$/\1/p; s/ is already released$//p'
  cd "$root"
}

expect() {
  local want=$1
  shift
  local got
  got=$(releases "$@")
  [ "$got" = "$want" ] || { echo "release of $*: got '$got', want '$want'" >&2; exit 1; }
}

expect v3.10.0 v3.10.0 3.10.0 'docs: explain'
expect v3.10.0 v3.10.0 3.10.0 'ci: cache the build' 'chore: tidy'
expect v3.10.1 v3.10.0 3.10.0 'fix: repair'
expect v3.10.1 v3.10.0 3.10.0 'perf(display): skip unchanged monitors'
expect v3.10.1 v3.10.0 3.10.0 'build(deps): bump the crates group with 5 updates (#33)'
expect v3.10.0 v3.10.0 3.10.0 'ci(deps): bump actions/checkout from 7.0.0 to 7.0.1'
expect v3.11.0 v3.10.0 3.10.0 'feat: add' 'fix: repair'
expect v4.0.0 v3.10.0 3.10.0 'feat(config)!: drop the legacy format'
expect v4.0.0 v3.10.0 3.10.0 $'fix: repair\n\nBREAKING CHANGE: new config format'
# A version raised by hand is kept: the state of this repository at v3.10.0, Cargo.toml at 3.10.2
expect v3.10.2 v3.10.0 3.10.2 'fix: SIGTERM, SIGINT and SIGHUP no longer crash the app'

# A real run: the bump commit touches only the package's version, in both files, then the tag is
# pushed and release.yml dispatched on it
mkdir "$tmp/bin"
printf '#!/bin/sh\necho "$*" > "%s/gh-call"\n' "$tmp" > "$tmp/bin/gh"
chmod +x "$tmp/bin/gh"
repo v3.10.0 3.10.0
git commit -qm 'feat: add' --allow-empty
PATH="$tmp/bin:$PATH" bash "$root/.github/release.sh" > /dev/null 2>&1
grep -qx 'version = "3.11.0"' Cargo.toml
[ "$(git show HEAD:Cargo.lock | grep -c 'version = "3.11.0"')" = 1 ]
git show HEAD:Cargo.lock | grep -A1 'name = "polling"' | grep -qx 'version = "3.10.0"'
[ "$(git log -1 --format=%s)" = 'chore(release): v3.11.0' ]
git -C "$tmp/remote.git" rev-parse -q --verify refs/tags/v3.11.0 > /dev/null
[ "$(git -C "$tmp/remote.git" rev-parse main)" = "$(git rev-parse HEAD)" ]
[ "$(cat "$tmp/gh-call")" = 'workflow run release.yml --ref v3.11.0' ]
cd "$root"

echo "release: ok"
