# Firefox Add-ons Publishing Guide

The sqz Firefox extension compresses context in ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. Published to [Firefox Add-ons (AMO)](https://addons.mozilla.org/).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-pack`
- A Firefox Add-ons developer account (free)
- `web-ext` CLI: `npm install -g web-ext`

## Build the extension

### 1. Compile the WASM module

From the repository root, the same way as for [Chrome](chrome-web-store.md):

```sh
wasm-pack build sqz-wasm --release --target no-modules --out-dir ../extension/wasm-pkg
cp extension/wasm-pkg/sqz_wasm.js extension/wasm-pkg/sqz_wasm_bg.wasm extension/
```

### 2. Build the Firefox extension

```sh
sh extension-firefox/build.sh
```

This copies the shared content scripts, the popup HTML and the two WASM files from `extension/` into `extension-firefox/`, and writes `extension-firefox/manifest.json` from the Chrome manifest with the gecko settings added and the background as `"scripts": ["sqz_wasm.js", "background.js"]`, so the `no-modules` glue defines `wasm_bindgen` before `background.js` runs.

### 3. Test locally

```sh
cd extension-firefox
web-ext run
```

Or load manually: `about:debugging` > This Firefox > Load Temporary Add-on > select `extension-firefox/manifest.json`.

## Key differences from Chrome

| | Chrome | Firefox |
|---|---|---|
| Manifest | `"service_worker": "background.js"` | `"scripts": ["sqz_wasm.js", "background.js"]` |
| API namespace | `chrome.*` | `browser.*` (promise-based) |
| Extension ID | Auto-assigned | Set in `browser_specific_settings.gecko.id` (`sqz@sqz-dev`) |
| Min version | N/A | `strict_min_version: "140.0"` |

## Package for submission

```sh
cd extension-firefox
web-ext build
# writes the package zip to web-ext-artifacts/
```

Or manually, leaving out the old package zip kept in that directory:

```sh
cd extension-firefox
zip -r ../sqz-firefox.xpi . -x "*.DS_Store" -x "build.sh" -x "*.zip"
```

## Publish

1. Go to [Firefox Add-ons Developer Hub](https://addons.mozilla.org/developers/)
2. Click "Submit a New Add-on"
3. Upload the `.xpi` or `.zip` file
4. Fill in listing details: description, screenshots, categories
5. Submit for review (typically 1-3 days for listed, instant for self-hosted)

## CI publishing (GitHub Actions)

Use `web-ext sign`:

```yaml
- name: Publish to Firefox Add-ons
  run: |
    cd extension-firefox
    web-ext sign --api-key=${{ secrets.AMO_API_KEY }} --api-secret=${{ secrets.AMO_API_SECRET }}
```

## Verify

After approval, users install from:
`https://addons.mozilla.org/firefox/addon/sqz/`
