#!/usr/bin/env sh
# Build the Firefox extension by copying shared assets from the Chrome extension
# and generating a Firefox-compatible manifest.json from the Chrome one.
# Run from the repo root: sh extension-firefox/build.sh

set -eu

CHROME_DIR="extension"
FIREFOX_DIR="extension-firefox"

echo "[sqz] Building Firefox extension..."

# --- Generate Firefox manifest.json from Chrome manifest ---
# Reads the Chrome manifest and applies Firefox-specific transforms:
#   1. Replace "service_worker" background with "scripts" array
#   2. Add browser_specific_settings for gecko
if command -v python3 >/dev/null 2>&1; then
  python3 -c "
import json, sys

with open('$CHROME_DIR/manifest.json') as f:
    m = json.load(f)

# Firefox MV3 uses background.scripts instead of service_worker
bg = m.get('background', {})
if 'service_worker' in bg:
    # Include WASM glue as a background script so it's available to background.js
    m['background'] = {'scripts': ['sqz_wasm.js', bg['service_worker']]}

# Add gecko settings
m['browser_specific_settings'] = {
    'gecko': {
        'id': 'sqz@sqz-dev',
        'strict_min_version': '140.0',
        'data_collection_permissions': {
            'required': ['none'],
            'optional': []
        }
    }
}

with open('$FIREFOX_DIR/manifest.json', 'w') as f:
    json.dump(m, f, indent=2)
    f.write('\n')
"
  echo "[sqz] Generated manifest.json from Chrome manifest"
else
  echo "[sqz] WARNING: python3 not found, skipping manifest generation"
fi

# Copy shared content scripts
mkdir -p "$FIREFOX_DIR/content"
cp "$CHROME_DIR/content/common.js"     "$FIREFOX_DIR/content/"
cp "$CHROME_DIR/content/chatgpt.js"    "$FIREFOX_DIR/content/"
cp "$CHROME_DIR/content/claude.js"     "$FIREFOX_DIR/content/"
cp "$CHROME_DIR/content/gemini.js"     "$FIREFOX_DIR/content/"
cp "$CHROME_DIR/content/grok.js"       "$FIREFOX_DIR/content/"
cp "$CHROME_DIR/content/perplexity.js" "$FIREFOX_DIR/content/"

# Copy popup HTML (popup.js is Firefox-specific, don't overwrite)
mkdir -p "$FIREFOX_DIR/popup"
cp "$CHROME_DIR/popup/popup.html" "$FIREFOX_DIR/popup/"

# Copy WASM artifacts if they exist
if [ -f "$CHROME_DIR/sqz_wasm_bg.wasm" ]; then
  cp "$CHROME_DIR/sqz_wasm_bg.wasm" "$FIREFOX_DIR/"
  cp "$CHROME_DIR/sqz_wasm.js"      "$FIREFOX_DIR/"
fi

echo "[sqz] Firefox extension built in $FIREFOX_DIR/"
echo "[sqz] To test: about:debugging > This Firefox > Load Temporary Add-on > select $FIREFOX_DIR/manifest.json"
echo "[sqz] To package: cd $FIREFOX_DIR && zip -r ../sqz-firefox.xpi ."
