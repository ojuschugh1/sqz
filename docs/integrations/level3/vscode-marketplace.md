# VS Code Marketplace Publishing Guide

The sqz VS Code extension is published to the [VS Code Marketplace](https://marketplace.visualstudio.com/) under the `sqz` publisher.

## Prerequisites

```sh
npm install -g @vscode/vsce
```

Log in with your publisher token (create one at https://marketplace.visualstudio.com/manage):

```sh
vsce login sqz
```

## Required `package.json` fields

The following fields must be present and accurate before publishing:

```json
{
  "name": "sqz",
  "displayName": "sqz — Context Intelligence",
  "description": "Compress and manage LLM context in VS Code.",
  "version": "0.1.0",
  "publisher": "sqz",
  "engines": { "vscode": "^1.85.0" },
  "categories": ["Other"],
  "repository": {
    "type": "git",
    "url": "https://github.com/ojuschugh1/sqz"
  },
  "license": "MIT",
  "icon": "images/icon.png"
}
```

Key rules:
- `publisher` must match your verified Marketplace publisher ID
- `icon` must be a 128×128 PNG
- `repository` is required for the Marketplace listing
- `version` must be bumped for each publish (semver)

## Package the extension

```sh
cd vscode-extension
npm install
vsce package
# produces sqz-0.1.0.vsix
```

Inspect the package contents before publishing:

```sh
vsce ls
```

## Publish

```sh
vsce publish
```

Or publish a specific version:

```sh
vsce publish minor   # bumps minor version and publishes
```

To publish a pre-release:

```sh
vsce publish --pre-release
```

## CI publishing (GitHub Actions)

Store your PAT as `VSCE_PAT` in repository secrets, then:

```yaml
- name: Publish to VS Code Marketplace
  run: vsce publish -p ${{ secrets.VSCE_PAT }}
  working-directory: vscode-extension
```

## Verify

After publishing, the extension appears at:
`https://marketplace.visualstudio.com/items?itemName=sqz.sqz`

Users can install with:

```sh
code --install-extension sqz.sqz
```
