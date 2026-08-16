# Distribution artifacts

Files for listing sqz-mcp in MCP registries. Nothing here publishes
automatically; each registry needs a one-time manual submission, documented
below.

## Official MCP Registry (registry.modelcontextprotocol.io)

`mcp-registry/server.json` is the metadata file. The registry hosts metadata
only; the actual package it points at is the `sqz-mcp` crate on crates.io.

Ownership verification: the registry fetches the crate README from crates.io
and looks for a visible `mcp-name: io.github.ojuschugh1/sqz` token. The main
README (which the crate ships) contains that token in its Links section, but
crates.io only sees it after a crate version published with the token is
live. So publish after the next crates.io release, not before.

Steps:

1. Release a new version to crates.io (the normal release flow; the crate
   README now carries the mcp-name token).
2. Bump both `version` fields in `mcp-registry/server.json` to match.
3. Install the publisher CLI: `brew install mcp-publisher`
4. `cd distribution/mcp-registry`
5. `mcp-publisher login github` (device-code flow; the io.github.ojuschugh1
   namespace comes from GitHub auth)
6. `mcp-publisher publish`
7. Verify: `curl "https://registry.modelcontextprotocol.io/v0/servers?search=sqz"`

The npm package (`sqz-cli`) already carries the matching `mcpName` field in
its package.json, but it is deliberately not listed in server.json: the
package ships two binaries (`sqz`, `sqz-mcp`) and neither matches the
package name, so a client running the registry-standard `npx sqz-cli` cannot
resolve which binary to run. Cargo installs a real `sqz-mcp` binary and has
no such ambiguity.

After the Docker image below is published to ghcr.io, an OCI package entry
can be appended to `packages` in server.json:

```json
{
  "registryType": "oci",
  "identifier": "ghcr.io/ojuschugh1/sqz-mcp:VERSION",
  "transport": { "type": "stdio" }
}
```

## Docker image (docker/Dockerfile)

Builds a minimal Alpine image whose entrypoint is `sqz-mcp` (stdio). Carries
the `io.modelcontextprotocol.server.name` label the MCP Registry uses for
OCI ownership verification.

```sh
# from repo root
docker build -f distribution/docker/Dockerfile -t ghcr.io/ojuschugh1/sqz-mcp:latest .
docker run -i --rm ghcr.io/ojuschugh1/sqz-mcp
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | docker run -i --rm ghcr.io/ojuschugh1/sqz-mcp
```

Not yet verified with a real build (no Docker on the dev machine); test the
three commands above before pushing the image or submitting anywhere.

## Docker MCP catalog (hub.docker.com/mcp)

`docker/server.yaml` is a draft entry for github.com/docker/mcp-registry.
Submission is a PR to their repo:

1. Fork docker/mcp-registry and clone it.
2. `mkdir servers/sqz && cp distribution/docker/server.yaml servers/sqz/`
3. Add `commit: <release commit SHA>` under `source:`.
4. From their repo: `task build -- sqz` and `task catalog -- sqz` to verify.
5. Open the PR.

Two things to know before submitting:

- Their CONTRIBUTING.md asks that the server license "allows people to
  consume it (MIT or Apache 2 are great, GPL is not)". sqz is ELv2, which
  permits free use but is not OSI-approved; whether they accept it is their
  call. Worst case the entry is rejected and the plain `docker run` path
  above still works.
- Docker builds and hosts the image themselves under `mcp/sqz` once merged;
  the ghcr.io image is independent of that.
