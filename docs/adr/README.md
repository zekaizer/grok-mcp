# Architecture Decision Records

ADRs capture **why** we made load-bearing choices. The public tool contract is
[`../tool_spec.md`](../tool_spec.md).

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-implementation-language-and-stack.md) | Implementation language and stack | Accepted |
| [0003](0003-xai-authentication.md) | xAI authentication and credential storage | Accepted |
| [0004](0004-tool-surface-token-offload.md) | Tool surface for host token offload | Accepted |
| [0005](0005-transport-and-deployment-phases.md) | Transport and deployment phases | Accepted |
| [0006](0006-http-transport-and-front-door-auth.md) | Streamable HTTP + front-door OAuth | Accepted |
| [0007](0007-deployment-and-configuration-posture.md) | systemd + single env file deploy | Accepted |

## Rules

- Nygard sections: Status / Context / Decision / Consequences.
- Write **before** implementing the decision; ship ADR with the code that enacts it.
- `Accepted` ADRs are immutable. Supersede with a new ADR rather than editing history.
- Write an ADR only for: library/framework choice, public API/tool contract changes, hard-to-reverse architecture, or non-functional deployment/security posture.
