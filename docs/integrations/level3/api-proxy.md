# API proxy

`sqz proxy` starts a local HTTP endpoint that compresses an OpenAI, Anthropic or Google AI request body and sends it back. It does not forward requests: nothing reaches the provider, and an API client pointed at it as its base URL gets the compressed request back instead of a model response. Use it to see what sqz would change in a request, or to compress a body in your own code before you send it to the provider yourself.

No release has forwarded requests, though 0.5.0 through 1.9.0 printed `OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL` examples when the endpoint started.

## Start it

```sh
sqz proxy --port 8080
```

`--port` (default 8080) is the only option. The endpoint listens on 127.0.0.1 and runs until you stop it. It handles one connection at a time and takes each request from one read of at most 1 MB. A body that has not fully arrived when that read returns is cut short: it comes back unparsed, as a string in `compressed_body`, or the client gets a connection error and no complete answer. Whether and where a body is cut depends on timing, not on a fixed size: on macOS, bodies of 60 KB to 160 KB were sometimes cut and sometimes not, no body of 200 KB or more came back whole, and one 200 KB body was cut after 16 KB.

## Send it a request

POST the request body to a path that names its format:

| Path contains | Format | Example path |
|---|---|---|
| `/chat/completions` | OpenAI | `/v1/chat/completions` |
| `/messages` | Anthropic | `/v1/messages` |
| `/generateContent` | Google AI | `/v1beta/models/gemini-2.5-flash/generateContent` |

Google AI's own `models/<model>:generateContent` path has no `/` before `generateContent`, so the endpoint answers it with 404.

```sh
curl -s http://127.0.0.1:8080/v1/chat/completions --data-binary @request.json
```

The answer is a JSON object, with the estimated saving in an `x-sqz-tokens-saved` header. For a 13-message OpenAI request with two long older turns and a 30-row JSON tool result:

```json
{
  "compressed_body": { "messages": [ ... ], "model": "gpt-4o" },
  "compressed_tokens": 334,
  "messages_compressed": 2,
  "messages_summarized": 2,
  "original_tokens": 1048,
  "reduction_pct": "68.1%",
  "sqz_proxy": true
}
```

`compressed_body` is the request with sqz's changes. The token figures are estimates.

## What it changes

- OpenAI: `system` and `tool` messages go through the sqz compression pipeline, and `user` and `assistant` messages before the 10 most recent are summarized.
- Anthropic: the `system` prompt and `tool_result` blocks go through the pipeline, and `user` and `assistant` messages before the 10 most recent are summarized.
- Google AI: the `system_instruction` parts and the text parts of `contents` go through the pipeline, except that long text parts before the 10 most recent contents are summarized instead.

A summary keeps the first and last line of a message that has more than three lines and about 200 characters or more, with `[... N lines, M chars ...]` between them. Only string content is changed; for Anthropic that includes the text of `tool_result` blocks. The rest of the body comes back as it was sent, though the order of keys may change.

## Other answers

- `/` and `/health`, with any method: `{"status":"ok","service":"sqz-proxy"}`.
- Any other path with a method other than POST: 405.
- A POST to a path without one of the strings above: 404.
- A body that is not JSON: sent back unchanged, as a string in `compressed_body`, with a line on stderr.
