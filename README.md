<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft-libs-plugins

Plugin-authoring libraries for [loft](https://github.com/loft-lang/loft) — each package published
to the registry under its own name.

Per the chunked-repo design in
[loft's lib_plans/12-library-extraction/](https://github.com/jjstwerff/loft/blob/main/doc/claude/lib_plans/12-library-extraction/README.md)
§ Chunk grouping. A plugin runtime is its own domain — not networking, not documents — so it gets
its own chunk rather than living in `loft-libs-net`.

## Packages

| Subdir | Package | Status |
|---|---|---|
| [`pluginabi/`](pluginabi/) | `pluginabi` — the plugin call protocol | v0.1.0 |

## `pluginabi` — what it is, and what it deliberately is not

A host drives a plugin through **six pure operations**: `initial_state`, `apply_op`, `make_op`,
`render`, `snapshot`, `load_snapshot`. This package is the **frame vocabulary** for those calls,
and nothing else — request/reply encoding as canonical CBOR, a closed operation set, and one
front-door validator.

**It is not a runtime, an ABI, or a sandbox.** That is the point. A loft program compiled to wasm
already exposes a fixed host boundary (`loft_io.loft_host_input_len` / `_copy` in,
`loft_host_print` / `loft_host_output` out) that a host pumps, and `host_input` / `host_output`
are per-target by design — stdin/stderr natively, the JS host's queue in a browser. So a plugin
needs no generated shim, no named wasm exports, and no embedded wasm engine: it needs the two
sides to agree on **what a frame means**. That is all this is.

The design that led here — including the three drafts that built a wasmtime runtime, a shim
generator and fuel metering before discovering loft already had the pieces — is written up in the
[consumer project's `doc/PLUGIN_RUNTIME.md`](https://github.com/jjstwerff/zero-trust-shared-files/blob/main/doc/PLUGIN_RUNTIME.md).

### Design choices worth knowing

- **An unknown operation is rejected, never guessed at.** A host and plugin disagreeing about the
  vocabulary is version skew, not a request to interpret.
- **`state` is always present, empty rather than absent.** A plugin never has to distinguish
  "absent" from "empty" — that ambiguity yields a plausible wrong answer instead of an error.
- **A failure carries a code, not a message.** A free-form string from a plugin would be a channel
  out of the sandbox that is neither render output nor a value the host asked for.
- **Replies fail closed.** Anything that is not a well-formed success — a truncated frame, garbage,
  even a *request* frame — reads as failure, so a garbled reply can never be read as success.

### Sandboxing

This package makes no safety claims and needs to make none: loft's own
[@PLN86 admission control](https://github.com/jjstwerff/loft/blob/main/doc/claude/SANDBOX.md)
proves a plugin total **at load** — bounded loops, acyclic recursion, an allow-listed capability
set, and a declared data envelope — so a host that admits a plugin before running it needs no
runtime guard. `pluginabi` itself is admissible at **O(1)**, so it costs a plugin nothing against
its declared budget.

## Usage

```toml
[dependencies]
pluginabi = ">=0.1.0"
```

```loft
use pluginabi;

pub fn dispatch(frame: vector<u8>) -> vector<u8> {
  bad = check_request(frame);            // decode, then vocabulary — the single front door
  if bad != "" { return reply_err(bad); }
  op = req_op(frame);
  st = req_state_b64(frame);             // opaque to the protocol; the plugin owns its meaning
  if op == OP_INITIAL_STATE { return reply_ok(my_empty_state()); }
  // …
  return reply_err(ERR_UNKNOWN_OP);
}
```

## Versioning + tags

Each package versions independently; git tags use **`<package>-v<version>`**.

```sh
cd <package>/
# bump version in loft.toml
git tag <package>-v<version> && git push --tags
loft package
gh release create <package>-v<version> <package>-<version>.tar.gz --title "<package> <version>"
# then a PR against loft-lang/registry adding the version row
```

## License

LGPL-3.0-or-later — see [LICENSE](LICENSE).
