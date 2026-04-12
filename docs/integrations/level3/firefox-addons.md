# Firefox Add-ons Publishing Guide

The sqz Firefox extension compresses context in ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. Published to [Firefox Add-ons (AMO)](https://addons.mozilla.org/).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-pack`
- A Firefox Add-ons developer account (free)
- `web-ext` CLI: `npm install -g web-ext`

## Build the extension

### 1. Compile the WASM module

```sh
cd sqz-wasm
wasm-pack build --target web --out-dir ../extension/pkg
```

### 2. Build the Firefox extension

```sh
sh extension-firefox/build.sh
```

This copies shared content scripts and popup from the Chrome extension into `extension-firefox/`.

### 3. Test locally

```sh
cd extension-firefox
web-ext run
```

Or load manually: `about:debugging` > This Firefox > Load Temporary Add-on > select `extension-firefox/manifest.json`.

## Key differences from Chrome

| | Chrome | Firefox |
|---|---|---|
| Manifest | `"service_worker": "background.js"` | `"scripts": ["background.js"]` |
| API namespace | `chrome.*` | `browser.*` (promise-based) |
| Extension ID | Auto-assigned | Set in `browser_specific_settings.gecko.id` |
| Min version | N/A | `strict_min_version: "109.0"` (MV3 support) |

## Package for submission

```sh
cd extension-firefox
web-ext build
# produces web-ext-artifacts/sqz_context_compression-0.1.0.zip
```

Or manually:

```sh
cd extension-firefox
zip -r ../sqz-firefox.xpi . -x "*.DS_Store" -x "build.sh"
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
