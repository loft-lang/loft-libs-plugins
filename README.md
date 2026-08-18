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
| [`pluginabi/`](pluginabi/) | `pluginabi` — the plugin call protocol | v0.1.2 |

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

### The four contracts a signature does not carry

A frame is `vector<u8>` and every payload is `text`.  The signatures say bytes in / bytes out and
text in / text out, and never that the text must be base64, that a reply hands back `""` for two
opposite outcomes, or that the front door checks the envelope and not the letter.  Each row links
to a test that demonstrates the correct call and runs in CI — the code is the documentation, so
it cannot go stale.

| contract | worked example |
|---|---|
| **Every payload is BASE64 text**, not the bytes and not the string.  `request(op, "hello", "")` is not an error — it is silent truncation to `"hell"`, because only that prefix is a whole base64 quantum.  Encode at the boundary (`base64_encode` for text, `bytes_to_base64` for binary). | [`@PAB-001`](pluginabi/tests/worked-examples.loft) |
| **Only `reply_is_ok` classifies a reply.**  `reply_out_b64` answers `""` for a failure *and* for a success carrying an empty payload; `reply_err_code` answers `""` for a success.  Neither field is a verdict, in either direction — ask the verdict, then read the one field that outcome has. | [`@PAB-002`](pluginabi/tests/worked-examples.loft) |
| **`check_request` validates the ENVELOPE, and `""` is its pass** (it returns the code to reply *with*, so it reads backwards from a boolean guard).  A known op with payload bytes no plugin could load passes; an `arg` sent for an operation that reads none passes; and a *reply* frame handed to it reports `unknown-op`, because a reply decodes fine and simply has no `op`. | [`@PAB-003`](pluginabi/tests/worked-examples.loft) |
| **The shape of a whole exchange** — front door, dispatch, all six operations, a plugin refusing a well-formed request, and a host rejecting an op the plugin body never sees. | [`@PAB-004`](pluginabi/tests/pluginabi.loft) |

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
