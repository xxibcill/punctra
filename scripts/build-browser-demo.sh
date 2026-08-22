#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wasm_bindgen_version="0.2.127"
output_directory="$repository_root/apps/browser-demo/web/pkg"
wasm_artifact="$repository_root/target/wasm32-unknown-unknown/release/browser_demo.wasm"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "wasm-bindgen-cli $wasm_bindgen_version is required" >&2
  echo "install it with: cargo install wasm-bindgen-cli --version $wasm_bindgen_version --locked" >&2
  exit 1
fi

installed_version="$(wasm-bindgen --version | awk '{print $2}')"
if [[ "$installed_version" != "$wasm_bindgen_version" ]]; then
  echo "wasm-bindgen-cli $wasm_bindgen_version is required; found $installed_version" >&2
  exit 1
fi

cd "$repository_root"
cargo build -p browser-demo --release --target wasm32-unknown-unknown
mkdir -p "$output_directory"
wasm-bindgen \
  --target web \
  --out-dir "$output_directory" \
  --out-name browser_demo \
  "$wasm_artifact"

echo "built browser host bindings in $output_directory"
