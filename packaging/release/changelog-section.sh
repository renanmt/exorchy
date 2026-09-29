#!/usr/bin/env bash
# Print the CHANGELOG.md section of one version (its body, without the
# "## [X.Y.Z] - date" heading). Fails when the section is missing or empty,
# so the release workflow cannot publish a version without notes.
#   packaging/release/changelog-section.sh 0.4.0
set -euo pipefail

version=${1:?usage: changelog-section.sh X.Y.Z}
version=${version#v}
changelog="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/CHANGELOG.md"

section=$(awk -v v="$version" '
  /^## \[/ { if (found) exit; if (index($0, "## [" v "]") == 1) { found = 1; next } }
  found { print }
' "$changelog")

# Trim blank lines at both ends.
section=$(printf '%s\n' "$section" | sed -e '/./,$!d' | tac | sed -e '/./,$!d' | tac)
if [[ -z $section ]]; then
  echo "CHANGELOG.md has no section for $version: add \"## [$version] - $(date +%F)\" with what changed." >&2
  exit 1
fi
printf '%s\n' "$section"
