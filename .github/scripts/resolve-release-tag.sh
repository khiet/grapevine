#!/usr/bin/env bash
# Exports TAG and VERSION to later steps of the calling release job.
#
# Reads EVENT (the triggering event name) and, on a manual rebuild, INPUT_TAG.
# Must run from a checkout of the release commit: on a push the tag comes from
# the release-please files in the working tree.
#
# A manual rebuild names the tag directly; a push derives it from the manifest
# and config rather than a release-please action output, whose per-package
# output key is namespaced by path and came back empty on an action re-run,
# which blanked the tag and broke the upload. Both mirror release-please's own
# tag format, '<package-name>-v<version>'.
set -euo pipefail

if [ "$EVENT" = "workflow_dispatch" ]; then
  TAG="$INPUT_TAG"
  # Strip the '<package-name>-v' prefix for the changelog lookup.
  VERSION="${TAG#*-v}"
else
  NAME="$(jq -r '.packages["."]["package-name"]' release-please-config.json)"
  VERSION="$(jq -r '."."' .release-please-manifest.json)"
  TAG="${NAME}-v${VERSION}"
fi

echo "TAG=$TAG" >> "$GITHUB_ENV"
echo "VERSION=$VERSION" >> "$GITHUB_ENV"
