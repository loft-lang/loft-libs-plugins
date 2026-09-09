<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# pluginabi — six operations, as frames on a channel both sides already have

A host drives a plugin through six pure operations — `initial_state`, `apply_op`, `make_op`,
`render`, `snapshot`, `load_snapshot`. This package is the **frame vocabulary** for those
calls: request and reply as canonical CBOR, a closed operation set, and one front-door
validator.

**It is not a runtime, an ABI, or a sandbox**, and that is the design rather than a gap. A
loft program compiled to wasm already exposes a fixed host boundary that a host pumps, so
the two sides need no generated shim and no embedded engine — they need to agree on what a
frame *means*. That agreement is this file, and it is the only thing either end imports.

## Install

```sh
loft install pluginabi
```

```loft
use pluginabi;
```

## The smallest thing that does something

```loft
use pluginabi;
use crypto;

fn main() {
    // The host builds a request.  Every payload is BASE64 TEXT, never raw bytes.
    state = bytes_to_base64([1 as u8, 2 as u8, 255 as u8]);
    frame = pluginabi::request(pluginabi::OP_APPLY_OP, state, bytes_to_base64([0 as u8, 7 as u8]));

    // The plugin checks the envelope before reading the letter.
    println("check   : '{pluginabi::check_request(frame)}'  (empty means valid)");
    println("op      : {pluginabi::req_op(frame)}");
    println("state   : {pluginabi::req_state_b64(frame) == state}  (byte for byte)");

    // …and answers.
    ok = pluginabi::reply_ok(bytes_to_base64([9 as u8, 9 as u8]));
    println("ok?     : {pluginabi::reply_is_ok(ok)}   out={pluginabi::reply_out_b64(ok)}");
    bad = pluginabi::reply_err(pluginabi::ERR_PLUGIN);
    println("ok?     : {pluginabi::reply_is_ok(bad)}  err={pluginabi::reply_err_code(bad)}");
}
```

```
check   : ''  (empty means valid)
op      : apply_op
state   : true  (byte for byte)
ok?     : true   out=CQk=
ok?     : false  err=plugin-error
```

The `255` and the `0` in that payload are the point: they are not text, and they come back
unchanged. That is why the frames are canonical CBOR rather than a text protocol — plugin
payloads are opaque bytes whose meaning the plugin owns, and they have to survive the round
trip exactly.

## Four things the signatures cannot tell you

A frame is `vector<u8>` and every payload is `text`. So the types say bytes in / bytes out
and text in / text out, and never these. Each row is a well-typed call that answers something
other than it appears to, and each links to a test that is its worked example — the code is
the documentation, so it cannot go stale.

| | worked example |
|---|---|
| **Every payload is BASE64 text**, not the bytes and not the string. `request(op, "hello", "")` is not an error — it is silent truncation to `"hell"`, the only whole base64 quantum in it. Encode at the boundary: `base64_encode` for text, `bytes_to_base64` for binary. | [`@PAB-001`](tests/worked-examples.loft) |
| **Only `reply_is_ok` classifies a reply.** `reply_out_b64` answers `""` for a failure *and* for a success carrying an empty payload; `reply_err_code` answers `""` for a success. Neither field is a verdict. Ask the verdict, then read the one field that outcome has. | [`@PAB-002`](tests/worked-examples.loft) |
| **`check_request` validates the ENVELOPE, and `""` is its pass.** It returns the code to reply *with*, so it reads backwards from a boolean guard. A known op carrying payload bytes no plugin could load passes; a *reply* frame handed to it reports `unknown-op`, because a reply decodes fine and simply has no `op`. | [`@PAB-003`](tests/worked-examples.loft) |
| **The shape of a whole exchange** — front door, dispatch, all six operations, a plugin refusing a well-formed request, and a host rejecting an op the plugin body never sees. | [`@PAB-004`](tests/pluginabi.loft) |

## The four decisions behind the frames

- **An unknown operation is rejected, never guessed at.** A host and a plugin disagreeing
  about the vocabulary is version skew, and interpreting it is how skew becomes a plausible
  wrong answer. `valid_op` is the gate; `check_request` is the door it sits in.
- **`state` is always present — empty, never absent.** So a plugin never has to tell "absent"
  from "empty", which is exactly the distinction that produces a plausible wrong answer
  instead of an error.
- **A failure carries a code, not a message.** A free-form string from a plugin would be a
  channel out of the sandbox that is neither render output nor a value the host asked for.
  `ERR_UNKNOWN_OP`, `ERR_MALFORMED` and `ERR_PLUGIN` are the closed set; the host turns a
  code into words a person reads.
- **Replies fail closed.** A truncated frame, garbage, even a *request* frame, all read as
  failure — so a garbled reply can never be mistaken for a success.

## Dispatching, in full

```loft
use pluginabi;

pub fn dispatch(frame: vector<u8>) -> vector<u8> {
  bad = check_request(frame);            // decode, then vocabulary — the single front door
  if bad != "" { return reply_err(bad); }
  op = req_op(frame);
  st = req_state_b64(frame);             // opaque to the protocol; the plugin owns its meaning
  if op == OP_INITIAL_STATE { return reply_ok(my_empty_state()); }
  if takes_arg(op) { /* … read req_arg_b64(frame) … */ }
  // …
  return reply_err(ERR_UNKNOWN_OP);
}
```

`takes_arg` records which operations read the `arg` slot — `apply_op` and `make_op`, and no
others. It lives here rather than in each host so the two sides cannot drift: a host that
sends `arg` for `snapshot` is confused about the contract, and a plugin that read it would be
reading whatever the host happened to leave there.

## Sandboxing

This package makes no safety claims and needs to make none. loft's own admission control
proves a plugin total **at load** — bounded loops, acyclic recursion, an allow-listed
capability set, a declared data envelope — so a host that admits a plugin before running it
needs no runtime guard. `pluginabi` itself is admissible at **O(1)**, so it costs a plugin
nothing against its declared budget.

## Testing

```sh
cd pluginabi && loft test
```

- `tests/pluginabi.loft` — the frames round-trip, and `@PAB-004` drives a complete
  six-operation plugin end to end over nothing but frames. If the protocol were missing
  something a real plugin needs, that test could not be written, which is why it exists here
  rather than as field-by-field assertions.
- `tests/worked-examples.loft` — `@PAB-001` … `@PAB-003`, the call sites the doc comments in
  `src/pluginabi.loft` cite. Each is a real running test, so a citation cannot rot.

## Status

Stable and additive. Depends on `cbor` (the canonical-CBOR codec) and `crypto` (base64), and
on nothing else — a published protocol library must stand on published dependencies alone.

⚠ Every frame is a CBOR map, and building one currently leaks one store per KEY
([loft#1491](https://github.com/loft-lang/loft/issues/1491)). It is invisible without
`LOFT_STORES=warn` and harmless in a short run, but a host driving many frames accumulates
one per key per frame. The defect is in the language, not in `cbor` and not here — a `match`
arm that binds a collection local from a call and yields it does not release it — so nothing
in either library changes when it lands.

## License

LGPL-3.0-or-later.
