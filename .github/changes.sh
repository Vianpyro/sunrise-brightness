#!/usr/bin/env bash
# Adapted from github.com/Vianpyro/pals (.github/changes.sh).
# Prints "area=true|false" for every area of the paths file ($1), from the changed files on stdin.
# A script rather than a path-filter action: one less third-party action holding the token
# (tj-actions/changed-files, March 2025). ALL=1, or a change under .github/, selects every area.
# Usage: git diff --name-only BASE...HEAD | bash .github/changes.sh .github/paths >> "$GITHUB_OUTPUT"
set -euo pipefail

paths=$1
mapfile -t files
declare -A selected

while read -r area pattern; do
  [ -z "$area" ] || [[ $area == \#* ]] && continue
  : "${selected[$area]:=false}"
  [ -z "${ALL:-}" ] || selected[$area]=true
  for file in "${files[@]}"; do
    # shellcheck disable=SC2053 # $pattern is a pattern on purpose
    if [[ $file == .github/* ]] || [[ $file == $pattern ]]; then
      selected[$area]=true
      break
    fi
  done
done <"$paths"

for area in $(printf '%s\n' "${!selected[@]}" | sort); do
  echo "$area=${selected[$area]}"
done
