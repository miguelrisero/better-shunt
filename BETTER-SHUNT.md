# Better Shunt

Public downstream of [pleaseai/shunt](https://github.com/pleaseai/shunt), retaining
upstream history, authorship, and LICENSE. The executable and configuration remain
named `shunt`; DevPod owns opt-in installation and account setup.

Setup guide: [docs/setup-shunt.md](https://github.com/miguelrisero/ultraship/blob/main/docs/setup-shunt.md).

## Baseline and maintained changes

Upstream baseline: `0462fb5364908d21ae154ee1fc26baa590ded48b` (v0.49.1), merged into
fork `main`. Changes are deliberately pinned instead of deploying upstream main automatically.

- Conservative schema regex filtering: a pattern must pass the Python compatibility
  check and compile with `regex-lite`, which rejects lookaround and backreferences
  ([#492](https://github.com/pleaseai/shunt/issues/492)). It applies to eager,
  shim-revealed, native ToolSearch and inbound Responses-to-Chat tools
  ([#489](https://github.com/pleaseai/shunt/issues/489)). Original request data is not mutated.
- Progressive tool discovery and readable tool-reference history on non-Anthropic
  Messages providers ([#424](https://github.com/pleaseai/shunt/issues/424)). No arbitrary
  truncation of the callable catalog.
- `shunt init` starter example uses `gpt-6-sol`.
- `tests/compatibility_guards.rs` pins these guards, including the public Neon email pattern.
- Fork model policy for fork-owned guidance: no Grok subagent plugin, Grok setup
  guides or Grok example routes, and no `gpt-5.6-sol`/`gpt-5.6-luna` examples or agents.
  Codex examples use `gpt-6-sol` and `gpt-6-luna`. Upstream xAI/Grok provider code,
  its compiled blueprints and its reference tables remain unchanged.

Upstream v0.48.0 provides the Codex client identity (`0.156.0`), native `tool_search`
for `gpt-6-astra`/`gpt-6-sol`/`gpt-6-luna`, `claude-opus-5-5` discovery, non-strict
function tools and WebSocket test account-store isolation.

Upstream v0.49.0 and v0.49.1 add the buffer-and-replay routing lane for escalation and
advisor ([#652](https://github.com/pleaseai/shunt/issues/652)), `gated_idle_ms`
enforcement in the Anthropic adapter's non-streaming model rewrite
([#668](https://github.com/pleaseai/shunt/issues/668)) and across the Gemini, Responses
HTTP and Cursor whole-body reads ([#670](https://github.com/pleaseai/shunt/issues/670)),
in-stream `slow_down`/overload/policy error classification
([#661](https://github.com/pleaseai/shunt/issues/661)) and status handling on wrapped
Codex WebSocket error frames ([#675](https://github.com/pleaseai/shunt/issues/675)).
The fork carries no patch for any of them. #661 supersedes the upstream assertion that
only `rate_limit_exceeded` is a throttle, so that comment leaves
`tests/responses_translate.rs` by upstream change, not by a dropped fork patch.

## Build dependencies

Default builds use three non-optional git dependencies from
[NVIDIA-NeMo/Switchyard](https://github.com/NVIDIA-NeMo/Switchyard), pinned in `Cargo.lock`
to rev `3ddea9d30174ad835cf505a93617ba251c8eb8dd` (version 0.3.0, Apache-2.0):
`switchyard-libsy`, `switchyard-protocol` and `switchyard-translation`. The routing
boundary in `src/routing/` and `src/config/router/` uses them; no feature flag removes them.
The optional `prefill-router` crate from the same rev links libpython through `pyo3`.
Release builds do not enable `prefill-router`; CI builds it through `--all-features`.

## Upstream issue audit — 2026-09-27 (re-checked at v0.49.1; #489, #492, #424 and #450 all still open)

| Issue | Applicability and policy |
| --- | --- |
| [#489](https://github.com/pleaseai/shunt/issues/489) | Preventively fixed in the inbound Chat translator. Upstream v0.48.0 has no caller for this translator, and it does not explain a Claude-to-Astra failure. |
| [#492](https://github.com/pleaseai/shunt/issues/492) | Add real regex-lite parsing to the existing Python approximation; pin lookaround, malformed syntax and literal-preservation regressions. This is conservative, not a guarantee of equivalence with every backend. |
| [skuda lookaround fix](https://github.com/skuda/shunt/commit/87c138c25eb7d7647682660288e04714811ef57e) | Same failure class already fixed here. The exact public Neon email pattern is included in eager and revealed-schema regression tests. |
| [#424](https://github.com/pleaseai/shunt/issues/424) | Existing progressive-reveal patch renders tool references as text for our non-Anthropic Messages routes. Native Anthropic remains passthrough. |
| [#450](https://github.com/pleaseai/shunt/issues/450) | Do not strip empty arrays/null globally: these can be intentional tool inputs. Preserve optionality with non-strict schemas and test actual tool contracts. |
| [#493](https://github.com/pleaseai/shunt/issues/493) | Closed upstream; v0.48.0 isolates the account store in WebSocket tests. |
| [#539](https://github.com/pleaseai/shunt/issues/539), [#533](https://github.com/pleaseai/shunt/issues/533) | Upstream environment-variable test races. Run the suite with RUST_TEST_THREADS=1 in clean CI; no production auth changes. |
| [#402](https://github.com/pleaseai/shunt/issues/402) | Affects gateway-login's model picker; DevPod uses explicit local BASE_URL and preserves native options. |
| [#354](https://github.com/pleaseai/shunt/issues/354), [#401](https://github.com/pleaseai/shunt/issues/401) | Managed OIDC gateway surfaces are not enabled in the loopback workstation deployment. Review before enabling them. |

## Update and validation policy

Keep an `upstream` remote. Merge an upstream release tag into a branch from fork `main`
and open a pull request; never force-push `main`. Keep only the maintained changes above,
then run format, clippy, and the complete test suite with the CI flags, and run DevPod's
synthetic streaming, tool-discovery, continuation, effort and compaction probes before
updating its immutable commit pin. Merge that pull request with a merge commit so upstream
ancestry stays in `main`.
Do not run provider-account tests against a developer's real credentials. Hosted CI
uses an empty runner and no model secrets. Only CI is enabled here; upstream site,
wiki, coverage-upload and release automation are not inherited as fork deployments.

Release binaries are built with the Rust version in `.github/workflows/ci.yml`,
`cargo build --release --locked`, release opt-level 1, LTO disabled and debug symbols
disabled, without optional features.

Retire downstream patches when an upstream replacement passes the same regressions.
Never infer a provider's context window from a Claude model alias. `[1m]` is a client
hint and must only be offered where the selected upstream actually supports it.
