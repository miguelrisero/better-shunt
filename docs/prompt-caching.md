# Prompt cache behavior

Shunt preserves client-selected Anthropic cache controls rather than replacing
every request with a one-hour TTL. A gateway cannot reliably infer an agent's
lifetime or cost break-even from its model id: reviewers and drivers may use
the same model. Set policy at the Claude Code client:

```json
{"promptCacheTtl": "1h"}
```

For long-lived `driver--opus` and `driver--inherit` agent definitions, add:

```yaml
experimental:
  cacheTtl: 1h
```

Leave `subagentPromptCacheTtl` unset so short reviewers, writers, planners and
test runners normally retain 5m. Main settings require Claude Code v2.1.242+;
agent frontmatter requires v2.1.248+. Restart Claude Code after agent edits.
Global settings/environment TTLs override frontmatter, and usage-credit billing
can suppress the frontmatter's 1h choice. See [Claude Code's precedence rules](https://code.claude.com/docs/en/prompt-caching#choose-the-ttl-yourself).
Shunt forwards native Anthropic markers (including mixed TTLs) and the
`anthropic-beta` header, and leaves response usage intact. The protocol test
`messages_preserves_mixed_cache_ttls_beta_header_and_usage_breakdown` guards this
contract. It proves transport, not real provider cache reuse or savings.

Check a fresh main run and driver transcript separately for
`usage.cache_creation.ephemeral_1h_input_tokens > 0`. Verify a short reviewer
still writes 5m. A fully cached request can have no writes in either bucket.
Streaming telemetry adds `shunt.tokens` kinds `cache_creation_5m` and
`cache_creation_1h` only when the upstream reports those fields. They are
breakdowns of `cache_creation`, not extra input. Missing breakdowns remain
missing; cache hits alone do not establish which TTL created an entry.

## Other providers

| Route | Cache behavior and accounting |
| --- | --- |
| OpenAI Responses / Codex | Existing session cache keys remain stable. Reported `input_tokens_details.cached_tokens` and `cache_write_tokens` become Anthropic cache-read and cache-creation counters; no Anthropic TTL breakdown is fabricated. |
| Gemini / Code Assist / Antigravity | Reported `usageMetadata.cachedContentTokenCount` becomes `cache_read_input_tokens`, clamped to total prompt tokens. `input_tokens` contains the remaining uncached input. Both streaming final usage and non-streaming JSON expose the split. |
| Anthropic-compatible providers (including CCR) | Cache markers and returned usage pass through. Whether the upstream honors them is provider-specific. A model branded DeepSeek on Cloudflare does not establish DeepSeek API cache semantics. |

The public [OpenAI API docs](https://developers.openai.com/api/docs/guides/prompt-caching)
describe GPT-5.6+ `prompt_cache_options.ttl` separately from earlier models'
`prompt_cache_retention`. This does not establish that the ChatGPT subscription
backend accepts those fields. Shunt's Anthropic-to-Responses translator does not
force either option on Codex or map `1h` to an unrelated provider retention.

[Gemini API caching](https://ai.google.dev/gemini-api/docs/caching) distinguishes
automatic implicit caching from explicit cached-content resources, whose storage
is charged over time. Shunt creates no explicit cache resource and makes no TTL
guarantee for Code Assist or Antigravity. The change exposes cache usage whenever
their backend reports it. Absent reads can mean a miss or unreported usage.

An engineer's observed long idle gaps favor 1h for main sessions and drivers,
but short runs can cost more because 1h writes carry a higher price. The list-price
break-even model is not measured subscription-limit consumption. Compare similar
usage periods before and after, and repeat transcript verification after client,
plugin, or gateway upgrades. No periodic keepalive requests are introduced.
