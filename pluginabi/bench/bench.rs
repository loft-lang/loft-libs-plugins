// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// pluginabi-reference — the pure-Rust twin of the `pluginabi` package's performance pass
// (bench/bench.loft), one file built with `rustc -O`.  It computes the SAME workloads and
// prints the same rows, hash included: a row whose hash matches the loft build's is a
// like-for-like comparison, and only then is a routine's loft time judged against it
// (@FR-Perf-Weight).  No dependencies and no cleverness — plain idiomatic Rust, the speed an
// industry implementation reaches without effort, which is exactly what the bar should be.
//
//     rustc -O --edition=2021 bench/bench.rs -o bench/.build/stats_rs && bench/.build/stats_rs --n 2
//
// The library is src/pluginabi.loft over two dependencies, and this file carries the part of
// each that the frames use:
//   - the CBOR subset of the `cbor` package (canonical heads, byte and text strings, maps,
//     booleans on decode; the decoder enforces shortest-form heads and rejects trailing bytes, exactly
//     as `cbor::decode` does), written the way a Rust codec is: a decoder that BORROWS its
//     strings out of the frame and answers a `Result`, and a map encoder that orders its
//     entries through a `BTreeMap` keyed by the encoded key;
//   - the base64 of the `crypto` package's native crate (native/src/base64.rs), by hand.
// Where the loft API decodes a frame once per field it reads, the twin decodes once and
// borrows every field off the one map — the census's idiom — and answers the same texts.
// `black_box` guards each op's INPUT (the repetition number) and the sink — never anything
// inside a kernel.
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

const FNV_OFFSET: i64 = 2166136261;
const FNV_PRIME: i64 = 16777619;

const CHECK_FRAMES: usize = 32;
const CHECKS: usize = 2048;
const READ_FRAMES: usize = 8;
const READS: usize = 32;
const STATE_KIB: usize = 4096;
const REQ_PAYLOADS: usize = 16;
const REQUESTS: usize = 512;
const REQ_STATE: usize = 512;

const OPS: [&str; 7] =
    ["initial_state", "apply_op", "make_op", "render", "snapshot", "load_snapshot", "reset"];

fn fnv_bytes(h0: i64, t: &[u8]) -> i64 {
    let mut h = h0;
    for &b in t {
        h = ((h ^ b as i64) * FNV_PRIME) & 0xFFFF_FFFF;
    }
    h
}

struct Row {
    name: &'static str,
    iters: i64,
    us: i64,
    px: i64,
    hash: i64,
    sink: i64,
}

fn print_row(r: &Row) {
    let ns_op = r.us * 1000 / r.iters;
    let ns_px = if r.px > 0 { (r.us * 1000) as f64 / (r.iters * r.px) as f64 } else { 0.0 };
    println!("{}\t{}\t{}\t{}\t{}\t{:.3}\t{:x}", r.name, r.iters, r.us, ns_op, r.px, ns_px, r.hash);
}

fn timed<F: FnMut(i64) -> i64>(n: i64, mut f: F) -> (i64, i64) {
    let t0 = Instant::now();
    let mut sink = 0i64;
    for r in 0..n {
        sink = sink.wrapping_add(f(black_box(r)));
    }
    (t0.elapsed().as_micros() as i64, black_box(sink))
}

// ── base64 (crypto/native/src/base64.rs) ────────────────────────────

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |&b| u32::from(b));
        let b2 = chunk.get(2).map_or(0, |&b| u32::from(b));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn b64_decode(input: &str) -> Vec<u8> {
    fn val(c: u8) -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a' + 26),
            b'0'..=b'9' => u32::from(c - b'0' + 52),
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => 0,
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=' && b != b'\n').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let mut n = val(chunk[0]) << 18 | val(chunk[1]) << 12;
        if chunk.len() > 2 {
            n |= val(chunk[2]) << 6;
        }
        if chunk.len() > 3 {
            n |= val(chunk[3]);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

// ── the CBOR subset (cbor/src/cbor.loft) ────────────────────────────

#[allow(dead_code)]
enum Cbor<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Bytes(&'a [u8]),
    Text(&'a str),
    Array(Vec<Cbor<'a>>),
    Map(Vec<(Cbor<'a>, Cbor<'a>)>),
}

/// Canonical head: the major type plus the shortest-form argument.
fn head(out: &mut Vec<u8>, major: u8, arg: u64) {
    let m = major << 5;
    if arg < 24 {
        out.push(m | arg as u8);
    } else if arg < 256 {
        out.extend_from_slice(&[m | 24, arg as u8]);
    } else if arg < 65536 {
        out.push(m | 25);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg < 4294967296 {
        out.push(m | 26);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

fn enc_text(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 9);
    head(&mut out, 3, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
    out
}

fn enc_bytes(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 9);
    head(&mut out, 2, b.len() as u64);
    out.extend_from_slice(b);
    out
}

/// A canonical map: the entries ordered by their encoded key bytes.
fn encode_map(entries: BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() + 9);
    head(&mut out, 5, entries.len() as u64);
    for (k, v) in entries {
        out.extend_from_slice(&k);
        out.extend_from_slice(&v);
    }
    out
}

struct Malformed;

fn read_value<'a>(b: &'a [u8], pos: usize) -> Result<(Cbor<'a>, usize), Malformed> {
    let n = b.len();
    let first = *b.get(pos).ok_or(Malformed)?;
    let major = first >> 5;
    let info = first & 31;
    let mut p = pos + 1;
    let arg: u64 = match info {
        0..=23 => u64::from(info),
        24..=27 => {
            let w = 1usize << (info - 24);
            if p + w > n {
                return Err(Malformed);
            }
            if info == 27 && b[p] >= 128 {
                return Err(Malformed); // exceeds i64 range
            }
            let a = b[p..p + w].iter().fold(0u64, |a, &x| a << 8 | u64::from(x));
            let min = [24u64, 256, 65536, 4294967296][(info - 24) as usize];
            if a < min {
                return Err(Malformed); // not the shortest form
            }
            p += w;
            a
        }
        _ => return Err(Malformed), // reserved or indefinite
    };
    match major {
        0 => Ok((Cbor::Int(arg as i64), p)),
        1 => Ok((Cbor::Int(-(arg as i64) - 1), p)),
        2 | 3 => {
            let len = arg as usize;
            if p + len > n {
                return Err(Malformed);
            }
            let s = &b[p..p + len];
            let v = if major == 2 { Cbor::Bytes(s) } else { Cbor::Text(std::str::from_utf8(s).unwrap_or("")) };
            Ok((v, p + len))
        }
        4 => {
            let mut items = Vec::new();
            for _ in 0..arg {
                let (v, next) = read_value(b, p)?;
                items.push(v);
                p = next;
            }
            Ok((Cbor::Array(items), p))
        }
        5 => {
            let mut entries = Vec::new();
            for _ in 0..arg {
                let (k, next) = read_value(b, p)?;
                let (v, next) = read_value(b, next)?;
                entries.push((k, v));
                p = next;
            }
            Ok((Cbor::Map(entries), p))
        }
        7 => match info {
            20 => Ok((Cbor::Bool(false), p)),
            21 => Ok((Cbor::Bool(true), p)),
            22 => Ok((Cbor::Null, p)),
            _ => Err(Malformed),
        },
        _ => Err(Malformed),
    }
}

fn decode(b: &[u8]) -> Result<Cbor<'_>, Malformed> {
    let (v, next) = read_value(b, 0)?;
    if next != b.len() {
        return Err(Malformed);
    }
    Ok(v)
}

impl<'a> Cbor<'a> {
    fn get(&self, key: &str) -> Option<&Cbor<'a>> {
        match self {
            Cbor::Map(entries) => entries
                .iter()
                .find(|(k, _)| matches!(k, Cbor::Text(t) if *t == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }
    fn text(&self, key: &str) -> &'a str {
        match self.get(key) {
            Some(Cbor::Text(t)) => t,
            _ => "",
        }
    }
    fn bytes(&self, key: &str) -> &'a [u8] {
        match self.get(key) {
            Some(Cbor::Bytes(b)) => b,
            _ => &[],
        }
    }
}

// ── pluginabi.loft ──────────────────────────────────────────────────

const ERR_UNKNOWN_OP: &str = "unknown-op";
const ERR_MALFORMED: &str = "malformed-frame";

fn valid_op(op: &str) -> bool {
    matches!(op, "initial_state" | "apply_op" | "make_op" | "render" | "snapshot" | "load_snapshot")
}

fn request(op: &str, state_b64: &str, arg_b64: &str) -> Vec<u8> {
    let mut m = BTreeMap::new();
    m.insert(enc_text("op"), enc_text(op));
    m.insert(enc_text("state"), enc_bytes(&b64_decode(state_b64)));
    m.insert(enc_text("arg"), enc_bytes(&b64_decode(arg_b64)));
    encode_map(m)
}

fn check_request(frame: &[u8]) -> &'static str {
    match decode(frame) {
        Err(Malformed) => ERR_MALFORMED,
        Ok(v) if !valid_op(v.text("op")) => ERR_UNKNOWN_OP,
        Ok(_) => "",
    }
}

/// op, state and arg off ONE decode — the state and argument answered as base64, as the
/// library answers them.
fn read_request(frame: &[u8]) -> (&str, String, String) {
    match decode(frame) {
        Ok(v) => (v.text("op"), b64_encode(v.bytes("state")), b64_encode(v.bytes("arg"))),
        Err(Malformed) => ("", String::new(), String::new()),
    }
}

// ── the workloads ───────────────────────────────────────────────────

fn payload(n: usize, v: i64) -> Vec<u8> {
    (0..n as i64).map(|k| (((k * 31 + v * 7) ^ (k >> 3)) & 255) as u8).collect()
}

fn payload_b64(n: usize, v: i64) -> String {
    b64_encode(&payload(n, v))
}

fn check_frames() -> Vec<Vec<u8>> {
    (0..CHECK_FRAMES)
        .map(|j| {
            let mut f = request(OPS[j % 7], &payload_b64(64, j as i64), &payload_b64(16, j as i64 + 100));
            if j & 7 == 7 {
                f.pop();
            }
            f
        })
        .collect()
}

fn bench_check(n: i64) -> Row {
    let frames = check_frames();
    let (us, sink) = timed(n, |r| {
        let mut sum = 0i64;
        for i in 0..CHECKS {
            sum += check_request(&frames[(i + r as usize) & 31]).len() as i64;
        }
        sum
    });
    let mut h = FNV_OFFSET;
    for i in 0..CHECKS {
        h = fnv_bytes(h, check_request(&frames[i & 31]).as_bytes());
    }
    Row { name: "check_request", iters: n, us, px: CHECKS as i64, hash: h, sink }
}

fn bench_read(n: i64) -> Row {
    let frames: Vec<Vec<u8>> = (0..READ_FRAMES)
        .map(|j| request(OPS[j % 6], &payload_b64(STATE_KIB, j as i64), &payload_b64(32, j as i64 + 50)))
        .collect();
    let (us, sink) = timed(n, |r| {
        let mut sum = 0i64;
        for i in 0..READS {
            let (op, state, arg) = read_request(&frames[(i + r as usize) & 7]);
            sum += (op.len() + state.len() + arg.len()) as i64;
        }
        sum
    });
    let mut h = FNV_OFFSET;
    for i in 0..READS {
        let (op, state, arg) = read_request(&frames[i & 7]);
        h = fnv_bytes(h, op.as_bytes());
        h = fnv_bytes(h, state.as_bytes());
        h = fnv_bytes(h, arg.as_bytes());
    }
    Row { name: "req_state_b64", iters: n, us, px: READS as i64, hash: h, sink }
}

fn bench_request(n: i64) -> Row {
    let states: Vec<String> = (0..REQ_PAYLOADS).map(|j| payload_b64(REQ_STATE, j as i64)).collect();
    let args: Vec<String> = (0..REQ_PAYLOADS).map(|j| payload_b64(32, j as i64 + 200)).collect();
    let (us, sink) = timed(n, |r| {
        let mut sum = 0i64;
        for i in 0..REQUESTS {
            let k = (i + r as usize) & 15;
            sum += request(OPS[i % 6], &states[k], &args[k]).len() as i64;
        }
        sum
    });
    let mut h = FNV_OFFSET;
    for i in 0..REQUESTS {
        h = fnv_bytes(h, &request(OPS[i % 6], &states[i & 15], &args[i & 15]));
    }
    Row { name: "request", iters: n, us, px: REQUESTS as i64, hash: h, sink }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut n = 20i64;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(1);
            i += 1;
        }
        i += 1;
    }
    if n < 1 {
        n = 1;
    }
    let t0 = Instant::now();
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let rows = [bench_check(n), bench_read(n), bench_request(n)];
    let mut sink = 0i64;
    for row in &rows {
        print_row(row);
        sink = sink.wrapping_add(row.sink);
    }
    println!("time: {}ms sink={}", t0.elapsed().as_millis(), sink);
}
