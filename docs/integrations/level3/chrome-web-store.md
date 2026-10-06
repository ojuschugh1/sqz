# Chrome Web Store Publishing Guide

The sqz Chrome extension compresses context in ChatGPT, Claude.ai, Gemini, Grok, and Perplexity. It is published to the [Chrome Web Store](https://chrome.google.com/webstore).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-pack`
- A Chrome Web Store developer account ($5 one-time fee)

## Build the extension

### 1. Compile the WASM module

From the repository root:

```sh
wasm-pack build sqz-wasm --release --target no-modules --out-dir ../extension/wasm-pkg
cp extension/wasm-pkg/sqz_wasm.js extension/wasm-pkg/sqz_wasm_bg.wasm extension/
```

`extension/background.js` loads `sqz_wasm.js` from the extension root with `importScripts` and calls the global `wasm_bindgen` it defines, which only the `no-modules` glue does, then fetches `sqz_wasm_bg.wasm` from the same place. wasm-pack writes a `.gitignore` into its output directory, so build into `extension/wasm-pkg/` and copy the two files rather than pointing `--out-dir` at `extension/` itself.

### 2. Load it unpacked

There is no bundling step: `extension/` is the extension as it ships. To test it, open `chrome://extensions`, turn on Developer mode, click **Load unpacked** and select `extension/`.

## Required `manifest.json` fields

```json
{
  "manifest_version": 3,
  "name": "sqz — Context Compression",
  "version": "0.5.0",
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
    "resources": ["sqz_wasm_bg.wasm", "sqz_wasm.js"],
    "matches": [
      "https://chatgpt.com/*",
      "https://chat.openai.com/*",
      "https://claude.ai/*",
      "https://gemini.google.com/*",
      "https://grok.com/*",
      "https://www.perplexity.ai/*"
    ]
  }]
}
```

The full manifest, with the content scripts for each site, is `extension/manifest.json`.

Key rules:
- `manifest_version` must be `3` (MV2 is deprecated)
- `version` must be bumped for each submission (semver, no pre-release labels)
- `description` is shown in the store listing (max 132 characters)
- All host permissions must be justified in the privacy disclosure

## Package for submission

Zip the contents of `extension/`, leaving out the tests and the wasm-pack output directory:

```sh
cd extension
zip -r ../sqz-extension-0.5.0.zip . -x "tests/*" -x "wasm-pkg/*" -x "*.DS_Store"
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
