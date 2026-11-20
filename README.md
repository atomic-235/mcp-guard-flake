# mcp-guard

A stdio gateway for one MCP server. It narrows the surface a client can see —
prompts, tools, resources — and refuses blocked names locally instead of
forwarding them.

```
client <-> mcp-guard <-> server
```

## Why

MCP clients expose everything a server advertises: tools, prompts, resources.
Some of that surface is not wanted in a given context, and neither the protocol
nor the client offers a way to hide a single prompt or tool. A gateway that
speaks the protocol is the only place the decision can be made without patching
the server.

## Install

```sh
nix profile add github:atomic-235/mcp-guard-flake
```

## Use

```sh
mcp-guard --server ai -- venice-mcp
```

The wrapper starts `venice-mcp`, reads the policy for `ai` from
`$XDG_CONFIG_HOME/opencode/mcp-guard/ai.json`, and relays every line in both
directions. Arguments after `--` are the server command and its arguments; the
child inherits the proxy's environment, so credential wrappers compose:

```sh
mcp-guard --server ai -- with-nono ai venice-mcp
```

| Flag | Meaning |
| - | - |
| `--server NAME` | Policy name; defaults to `mcp` in the audit log and lookup path |
| `--policy FILE` | Policy file; defaults to `$XDG_CONFIG_HOME/opencode/mcp-guard/<server>.json` |
| `--audit FILE` | Audit log; defaults to `$XDG_STATE_HOME/mcp-guard/audit.jsonl` |

A missing or unreadable policy logs a warning on stderr and relays unguarded,
so a missing file never blocks the client. The gateway is a preference filter;
the security boundary stays in the sandbox and credential layer.

## Policy

```jsonc
{
  // Absent means "no restriction for this category".
  "prompts": ["uncensored-research", "image-style-explorer"],
  "tools": ["venice_*", "read"],
  "resources": ["venice://styles"]
}
```

Each category that is present is an **allowlist**, so a prompt or tool the
server adds in a future release stays hidden until it is named here. Patterns
are whole-value globs: `*` matches any run of characters, everything else is
literal. An empty array means "nothing of that category".

## What it enforces

For each guarded category:

1. `initialize` — the matching server capability is removed when nothing of that
   category is allowed, so a compliant client never asks for the list.
2. `prompts/list`, `tools/list`, `resources/list`, `resources/templates/list` —
   the returned array is filtered to the allowlist.
3. `prompts/get`, `tools/call`, `resources/read` (and `resources/subscribe` /
   `unsubscribe`) — a name outside the allowlist is answered with JSON-RPC
   `-32602` and never reaches the server.

Everything else is relayed byte for byte: notifications, server-initiated
requests, non-JSON lines on stdout, and any response that did not need
rewriting. Responses are matched to their request by id value.

## Audit

Every decision appends one JSON line:

```json
{"ts":1763600000,"server":"ai","method":"prompts/list","name":"2/3 kept","decision":"filter"}
```

## Build

```sh
nix build .#static     # static musl binary, published as the release asset
nix build .#default    # the asset, fetched back
cargo test             # policy and framing tests
```

The release workflow builds `.#static`, publishes `mcp-guard-linux-x86_64`,
writes the hash into `version.json`, and verifies that `.#default` consumes the
published asset before committing.
