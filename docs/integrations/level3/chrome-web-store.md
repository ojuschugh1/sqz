# Chrome Web Store Publishing Guide

The sqz Chrome extension compresses context in ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. It is published to the [Chrome Web Store](https://chrome.google.com/webstore).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-pack`
- Node.js + webpack (for bundling content scripts)
- A Chrome Web Store developer account ($5 one-time fee)

## Build the extension

### 1. Compile the WASM module

```sh
cd sqz-wasm
wasm-pack build --target web --out-dir ../extension/pkg
```

This produces `sqz_wasm.js` and `sqz_wasm_bg.wasm` in `extension/pkg/`.

### 2. Bundle content scripts

```sh
cd extension
npm install
npm run build
# produces dist/ with bundled JS and the WASM file
```

The webpack config should copy `pkg/sqz_wasm_bg.wasm` into the output directory so it is accessible as a `web_accessible_resource`.

## Required `manifest.json` fields

```json
{
  "manifest_version": 3,
  "name": "sqz — Context Compression",
  "version": "0.1.0",
  "description": "Compresses LLM context in ChatGPT, Claude.ai, Gemini, Grok, and Perplexity.",
  "permissions": ["storage"],
  "host_permissions": [
    "https://chatgpt.com/*",
    "https://chat.openai.com/*",
    "https://claude.ai/*",
    "https://gemini.google.com/*",
    "https://grok.com/*",
    "https://www.perplexity.ai/*"
  ],
  "background": { "service_worker": "background.js" },
  "action": { "default_popup": "popup/popup.html" },
  "web_accessible_resources": [{
    "resources": ["pkg/sqz_wasm_bg.wasm", "pkg/sqz_wasm.js"],
    "matches": ["<all_urls>"]
  }]
}
```

Key rules:
- `manifest_version` must be `3` (MV2 is deprecated)
- `version` must be bumped for each submission (semver, no pre-release labels)
- `description` is shown in the store listing (max 132 characters)
- All host permissions must be justified in the privacy disclosure

## Package for submission

Zip the built extension directory (not the source):

```sh
cd extension/dist
zip -r ../sqz-extension-0.1.0.zip .
```

## Publish

1. Go to the [Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole)
2. Click **Add new item** and upload the zip
3. Fill in the store listing: description, screenshots (1280×800 or 640×400), promotional tile (440×280)
4. Set visibility to **Public** or **Unlisted**
5. Submit for review (typically 1-3 business days)

## CI publishing (GitHub Actions)

Use the [chrome-webstore-upload-cli](https://github.com/nicedoc/chrome-webstore-upload-cli):

```yaml
- name: Publish to Chrome Web Store
  run: npx chrome-webstore-upload-cli upload --source sqz-extension.zip --extension-id $EXT_ID
  env:
    EXTENSION_ID: ${{ secrets.CHROME_EXTENSION_ID }}
    CLIENT_ID: ${{ secrets.CHROME_CLIENT_ID }}
    CLIENT_SECRET: ${{ secrets.CHROME_CLIENT_SECRET }}
    REFRESH_TOKEN: ${{ secrets.CHROME_REFRESH_TOKEN }}
```

## Verify

After approval, users can install from:
`https://chrome.google.com/webstore/detail/sqz/<extension-id>`
