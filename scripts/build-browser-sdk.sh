#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_directory="$repository_root/target/npm"
viewer_manifest="$repository_root/apps/browser-demo/web/package.json"
viewer_package_version="$(
  node --input-type=module -e '
    import { readFileSync } from "node:fs";
    process.stdout.write(JSON.parse(readFileSync(process.argv[1], "utf8")).version);
  ' "$viewer_manifest"
)"

"$repository_root/scripts/build-browser-demo.sh"
mkdir -p "$artifact_directory"
find "$artifact_directory" -maxdepth 1 -type f \
  \( -name 'punctra-viewer-*.tgz' -o -name 'punctra-react-*.tgz' \) -delete
npm pack "$repository_root/apps/browser-demo/web" --pack-destination "$artifact_directory"
npm pack "$repository_root/packages/react" --pack-destination "$artifact_directory"
viewer_package_directory="$repository_root/apps/browser-demo/web/node_modules/@punctra/viewer"
rm -rf "$viewer_package_directory"
mkdir -p "$viewer_package_directory"
tar -xzf "$artifact_directory/punctra-viewer-$viewer_package_version.tgz" \
  -C "$viewer_package_directory" \
  --strip-components=1
node "$repository_root/scripts/generate-browser-sdk-reference.mjs"

echo "built browser SDK artifacts in $artifact_directory"
