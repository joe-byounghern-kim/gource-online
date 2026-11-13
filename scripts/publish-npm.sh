#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Publish the two npm tarballs of a release: the protocol first, then the CLI.
#   scripts/publish-npm.sh <artifact-dir> <version> [--dry-run]
# The release workflow runs this twice with the same artifacts: with --dry-run
# before the protected publish job, so a broken command fails before anyone is
# asked to approve, and without it in that job. A pre-release version gets the
# dist-tag beta, any other latest.
set -euo pipefail

dir=${1:?usage: publish-npm.sh <artifact-dir> <version> [--dry-run]}
version=${2:?usage: publish-npm.sh <artifact-dir> <version> [--dry-run]}
mode=${3:-}

# Absolute paths: npm reads a bare "out/name.tgz" as a GitHub "owner/repo" shorthand, not as a file.
dir=$(cd "$dir" && pwd)
protocol="$dir/gource-online-protocol-$version.tgz"
# upload-artifact keeps paths relative to the repository root.
cli="$dir/apps/cli/gource-online-$version.tgz"
test -f "$protocol"
test -f "$cli"

tag=latest
case "$version" in *-*) tag=beta ;; esac

case "$mode" in
  --dry-run) flags=(--dry-run) ;;
  '') flags=(--provenance) ;;
  *) echo "unknown option: $mode" >&2; exit 2 ;;
esac

npm publish "$protocol" --access public --tag "$tag" "${flags[@]}"
npm publish "$cli" --access public --tag "$tag" "${flags[@]}"
