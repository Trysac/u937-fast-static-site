# Project record

## Objective

Incremental static site generator with a hand-written CommonMark subset, strict templates and a dependency graph that rebuilds only the pages a change touches

## Scope

- In scope: the public API documented in [README.md](../README.md), documented and tested.
- Out of scope: anything that needs dependencies or I/O without an accepted ADR.

## Timeline

| Date | Event |
|---|---|
| 2026 | Repository created from the factory utility template. |

## Effort metrics

Measured values only; a value that has not been measured stays "pending measurement".

| Metric | Value |
|---|---|
| Tokens (input / output) | 5430331 / 73178 |
| Hours (human) | pending measurement |
| Cost (USD) | 1.7488 |

## Factory run u937 (measured from API usage)

| Date (UTC) | Phase | Model | Batch | Input | Output | Cache write | Cache read | Cost (USD) |
|---|---|---|---|---|---|---|---|---|
| 2026-11-17 | generation | google/gemini-4-argon | yes | 6204 | 35947 | 0 | 0 | 0.7438 |
| 2026-11-17 | fix-1 | google/gemini-3.8-flash | no | 380683 | 27827 | 0 | 2130368 | 0.5496 |
| 2026-11-17 | fix-2 | google/gemini-3.8-flash | no | 119660 | 6570 | 0 | 1512327 | 0.2278 |
| 2026-11-17 | audit | google/gemini-3.8-flash | no | 177806 | 1344 | 0 | 1078938 | 0.2193 |
| 2026-11-17 | docs | google/gemini-3.1-flash-lite | no | 24345 | 1490 | 0 | 0 | 0.0083 |

Total: $1.7488. Prices: vertex line, as measured by its adapter. Providers serve the current version of each model id, so the date tells which version answered; versions, launch stages and price sources per line are in the factory's model register (docs/MODELS.md).
