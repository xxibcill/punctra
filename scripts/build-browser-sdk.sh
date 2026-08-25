#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_directory="$repository_root/target/npm"

"$repository_root/scripts/build-browser-demo.sh"
mkdir -p "$artifact_directory"
find "$artifact_directory" -maxdepth 1 -type f \
  \( -name 'punctra-viewer-*.tgz' -o -name 'punctra-react-*.tgz' \) -delete
npm pack "$repository_root/apps/browser-demo/web" --pack-destination "$artifact_directory"
npm pack "$repository_root/packages/react" --pack-destination "$artifact_directory"
node "$repository_root/scripts/generate-browser-sdk-reference.mjs"

echo "built browser SDK artifacts in $artifact_directory"
