# API Proxy Mode (Planned)

> **Status: Not yet implemented.** This document describes the planned proxy feature for a future release. The `sqz proxy` subcommand is not available in v0.1.0.

sqz will act as a transparent HTTP proxy that sits between your application and the OpenAI, Anthropic, or Google AI APIs. Every request will pass through the sqz compression pipeline before being forwarded, reducing token usage without changing your application code.

## How it works

```
Your app  →  sqz proxy (localhost:8080)  →  OpenAI / Anthropic / Google AI
```

sqz intercepts the request body, compresses the `messages` context through the standard pipeline, then forwards the modified request to the upstream API. The response is returned unchanged.

## Start the proxy

```sh
sqz proxy --port 8080
```

By default sqz reads your API keys from the standard environment variables and forwards to the real endpoints.

## OpenAI

Point your OpenAI client at the sqz proxy by overriding the base URL:

```sh
export OPENAI_API_KEY=sk-...
sqz proxy --port 8080 --upstream https://api.openai.com
```

```python
import openai
client = openai.OpenAI(
    api_key="sk-...",
    base_url="http://localhost:8080/v1",
)
```

```js
import OpenAI from "openai";
const client = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY,
  baseURL: "http://localhost:8080/v1",
});
```

The proxy rewrites the `Authorization` header automatically if you set `OPENAI_API_KEY` in the environment.

## Anthropic

```sh
export ANTHROPIC_API_KEY=sk-ant-...
sqz proxy --port 8080 --upstream https://api.anthropic.com
```

```python
import anthropic
client = anthropic.Anthropic(
    api_key="sk-ant-...",
    base_url="http://localhost:8080",
)
```

sqz detects Anthropic's prompt cache headers and preserves cache boundaries during compression to maintain the 90% cache discount.

## Google AI (Gemini)

```sh
export GOOGLE_API_KEY=AIza...
sqz proxy --port 8080 --upstream https://generativelanguage.googleapis.com
```

```python
import google.generativeai as genai
genai.configure(
    api_key="AIza...",
    transport="rest",
    client_options={"api_endpoint": "http://localhost:8080"},
)
```

## Configuration file

Create `~/.config/sqz/proxy.toml` to set persistent options:

```toml
[proxy]
port = 8080
preset = "default"

[proxy.upstreams]
openai   = "https://api.openai.com"
anthropic = "https://api.anthropic.com"
google   = "https://generativelanguage.googleapis.com"

[proxy.compression]
# Inherit from the active preset; override specific stages here
strip_nulls = true
condense = true
```

## Notes

- The proxy is entirely local — no data leaves your machine except the compressed request forwarded to the upstream API.
- All compression happens in-process; latency overhead is typically under 5ms.
- Use `sqz proxy --dry-run` to log what would be compressed without forwarding requests.
