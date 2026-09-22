# Better Shunt

Private downstream of [pleaseai/shunt](https://github.com/pleaseai/shunt), retaining
upstream history, authorship, and LICENSE. The executable and configuration remain
named `shunt`; DevPod owns opt-in installation and account setup.

## Baseline and maintained changes

Upstream baseline: `e1de716a887124cbc3de276ea2a150a531c6998b` (v0.44.0).
Changes are deliberately pinned instead of deploying upstream main automatically.

- Conservative schema regex filtering on eager, shim-revealed, native ToolSearch,
  and inbound Responses-to-Chat tools. Original request data is not mutated.
- Explicit non-strict OpenAI/ChatGPT function tools preserve optional parameters.
- Progressive tool discovery and readable tool-reference history on non-Anthropic
  Messages providers. No arbitrary truncation of the callable catalog.
- WebSocket tests isolate the Codex account store and restore the previous override.
- Codex client identity pinned to openai/codex rust-v0.155.1, the floor for
  `gpt-6-sol` and `gpt-6-luna` (`minimal_client_version: 0.155.0`).
- Native Responses `tool_search` for the exact `gpt-6-astra`, `gpt-6-sol` and
  `gpt-6-luna` slugs, matching `supports_search_tool` in the Codex catalog.
- Discovery snapshot includes `claude-opus-5-5`.

## Upstream issue audit — 2026-09-12

| Issue | Applicability and policy |
| --- | --- |
| [#489](https://github.com/pleaseai/shunt/issues/489) | Preventively fixed in the inbound Chat translator. This translator is not wired to the proxy at this baseline and does not explain a Claude-to-Astra failure. |
| [#492](https://github.com/pleaseai/shunt/issues/492) | Add real regex-lite parsing to the existing Python approximation; pin lookaround, malformed syntax and literal-preservation regressions. This is conservative, not a guarantee of equivalence with every backend. |
| [skuda lookaround fix](https://github.com/skuda/shunt/commit/87c138c25eb7d7647682660288e04714811ef57e) | Same failure class already fixed here. The exact public Neon email pattern is included in eager and revealed-schema regression tests. |
| [#424](https://github.com/pleaseai/shunt/issues/424) | Existing progressive-reveal patch renders tool references as text for our non-Anthropic Messages routes. Native Anthropic remains passthrough. |
| [#450](https://github.com/pleaseai/shunt/issues/450) | Do not strip empty arrays/null globally: these can be intentional tool inputs. Preserve optionality with non-strict schemas and test actual tool contracts. |
| [#493](https://github.com/pleaseai/shunt/issues/493) | Fixed test account-store isolation. |
| [#539](https://github.com/pleaseai/shunt/issues/539), [#533](https://github.com/pleaseai/shunt/issues/533) | Upstream environment-variable test races. Run the suite with RUST_TEST_THREADS=1 in clean CI; no production auth changes. |
| [#402](https://github.com/pleaseai/shunt/issues/402) | Affects gateway-login's model picker; DevPod uses explicit local BASE_URL and preserves native options. |
| [#354](https://github.com/pleaseai/shunt/issues/354), [#401](https://github.com/pleaseai/shunt/issues/401) | Managed OIDC gateway surfaces are not enabled in the loopback workstation deployment. Review before enabling them. |

## Update and validation policy

Keep an `upstream` remote, fetch and review changes in a branch, run format, clippy,
and the complete test suite, then run DevPod's synthetic streaming, tool-discovery,
continuation, effort and compaction probes before updating its immutable commit pin.
Do not run provider-account tests against a developer's real credentials. Hosted CI
uses an empty runner and no model secrets. Only CI is enabled here; upstream site,
wiki, coverage-upload and release automation are not inherited as company deployments.

Retire downstream patches when an upstream replacement passes the same regressions.
Never infer a provider's context window from a Claude model alias. `[1m]` is a client
hint and must only be offered where the selected upstream actually supports it.
