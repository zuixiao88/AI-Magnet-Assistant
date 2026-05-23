#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT_DIR"

PRODUCT_NAME=$(node -e "console.log(require('./src-tauri/tauri.conf.json').productName)")
VERSION=$(node -e "console.log(require('./package.json').version)")
ARCH=$(uname -m)

echo "Packaging $PRODUCT_NAME v$VERSION (macOS $ARCH portable)"

if [[ "${SKIP_BUILD:-}" != "1" ]]; then
  npm run -s tauri build -- --bundles app
fi

APP_DIR="src-tauri/target/release/bundle/macos/${PRODUCT_NAME}.app"
PORTABLE_DIR="src-tauri/target/release/bundle/portable"
ZIP_NAME="${PRODUCT_NAME}_${VERSION}_macos26_${ARCH}-portable.app.zip"
ZIP_PATH="$PORTABLE_DIR/$ZIP_NAME"

mkdir -p "$PORTABLE_DIR"

if [[ ! -d "$APP_DIR" ]]; then
  echo "App bundle not found: $APP_DIR" >&2
  exit 1
fi

rm -f "$ZIP_PATH"
ditto -c -k --keepParent "$APP_DIR" "$ZIP_PATH"

echo "Created portable app archive: $ZIP_PATH"
ls -lh "$ZIP_PATH"
