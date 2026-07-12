# 1. Record architecture decisions

## Status

Accepted

## Context

grok-mcp is a new MCP server that bridges host agents (Claude Code, claude.ai)
to xAI's Responses API using SuperGrok subscription credentials. Before
implementation, several foundational choices must land: language and MCP SDK,
authentication and secret handling, the public tool surface, and a phased
transport story (local stdio first, remote HTTP later).

We need a durable record of *why* each choice was made so trade-offs stay visible
and decisions are not silently reversed.

## Decision

We will record architecturally significant decisions as Architecture Decision
Records (ADRs) using the Michael Nygard format under `docs/adr/`, one file per
decision named `NNNN-kebab-title.md` with monotonically increasing numbers.

- **Template:** `## Status` / `## Context` / `## Decision` (active voice,
  "We will …") / `## Consequences` (positive, negative, neutral).
- **Timing:** write the ADR before the code that implements it; include both in
  the same change set when code exists.
- **Scope — ADR required for:** library or framework choice, public MCP tool/API
  contract changes, hard-to-reverse module or deployment architecture, and
  security-relevant non-functional requirements.
- **Out of scope for ADRs:** routine refactors, bug fixes, and reversible
  implementation details — those belong in commits and code comments.
- **Immutability:** an `Accepted` ADR is not edited in place. Change the decision
  by superseding: set status to `Superseded by ADR-N` and add a new ADR.

## Consequences

- Positive: reviewers and future maintainers see intent next to the code.
- Positive: scope rule limits ADR spam.
- Negative: small process overhead on foundational changes.
- Neutral: end-user docs stay in `README.md`; tool schemas stay in
  `docs/tool_spec.md`.
