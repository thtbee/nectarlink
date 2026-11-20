# ADR 0010: Agent access through one action layer

- **Status:** Accepted
- **Date:** 2026-11-19

## Context

People want AI agents (Claude Code, Codex, Cursor, Gemini CLI, Antigravity,
VS Code and the like) to use NectarLink: read notifications and texts, send a
file to the phone, reply to a message. Agents reach tools in different ways.
IDE and chat agents speak MCP, shell agents run commands, and scripts use HTTP.

What agents could touch is as private as it gets: messages, calls, the
clipboard, files, and the user's phone. Text in those messages is written by
other people and can carry instructions aimed at the agent.

## Decision

One action layer, `core/nectarlink-actions`, defines every action once: its
name, input schema, permission scope, risk, confirmation rule and rate class.
One gateway checks every request, in this order: agent access on, the token,
the action, the client's scopes, read-only, the device and that device's own
permission, the rate limit, and the user's confirmation. Then it runs the
action and writes an activity row, whatever happened.

Three channels sit on top, and none of them reaches the node another way:

1. **MCP**, through `nectarlink mcp`, a stdio server that forwards to the
   running app. The main way for agents.
2. **The CLI**, `nectarlink do <action>`, with JSON output. Costs a shell
   agent nothing until it runs.
3. **The local HTTP API**, v2, for scripts and other apps.

Rules that hold on every channel:

- Off by default. Each agent gets its own token and scopes, named after the
  per-device permissions. A scope never beats a device's own permission.
- Sending messages, calling, deleting and similar actions are confirmed by the
  user in NectarLink's own window, bound to the exact arguments. The agent and
  its client can't answer that window.
- Text from outside is cleaned, capped, one-time codes are hidden, and results
  say which fields carry it.
- Pairing, permissions, tokens, keys, updates, wiping data and any unlock
  credential have no action at all.
- Remote agents (Grok and other cloud agents) are not supported for now. They
  need a public endpoint and give the user no approval step of their own.

## Alternatives

- **MCP only:** leaves out scripts and shell agents, which work better with a
  command, and loads every tool description into the agent's context.
- **CLI only:** chat-only and IDE agents can't use it, and it has no live data.
- **Screen control (computer use):** slow, unreliable and expensive when the
  app can be asked directly.
- **A2A:** for agents talking to agents, not for calling an app's actions.

## Consequences

- One place to review and test the security rules.
- The MCP library stays in one crate; the action crate is MPL-2.0 like the core,
  so the phone could use it later.
- A token is not a sandbox: anything running as the user can read token files.
  The confirmation window is the boundary, and the docs say so.
