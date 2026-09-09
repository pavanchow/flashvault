<img src="docs/logo.svg" alt="Flashvault logo" width="96">

# Flashvault: an LRU cache in Rust

Flashvault is a small, readable LRU cache in Rust with TTL expiry, a capacity bound, and live stats, built from scratch with no crate doing the eviction or expiry for you. A HashMap gives O(1) lookup, an intrusive doubly linked list gives O(1) recency tracking, and a slab of reusable slots keeps it allocation-light. The result is a cache small enough to read end to end and small enough to embed instead of pulling in a large dependency.

**[Live demo](https://pavanchow.github.io/flashvault/)** · MIT licensed · written in Rust

## What it does

- Capacity bound: a max entry count that is never exceeded.
- LRU eviction: when full, the least-recently-used entry is evicted first.
- TTL expiry: entries can carry a time to live in milliseconds. An expired entry is a miss on the next `get`, and is removed at that point.
- Recency on read: `get` moves the touched key to the front of the order, so hot keys survive.
- Live stats: hits, misses, evictions, expirations, and current size, plus a derived hit rate.
- Injectable clock: a `Clock` trait separates the real wall clock from a settable `TestClock`, so TTL behavior is testable without sleeping.
- O(1) get and put, and no panics on any input, including a capacity of zero.

## Usage

As a library:

```rust
use flashvault::{Cache, SystemClock};

let mut cache: Cache<String, SystemClock> = Cache::new(128, SystemClock);
cache.put("key".to_string(), "value".to_string(), Some(5_000)); // 5s TTL
assert_eq!(cache.get("key"), Some("value".to_string()));
println!("{:?}", cache.stats());
```

From the command line:

```sh
# one-shot set, one-shot get
cargo run -- set mykey myvalue --ttl-ms 5000
cargo run -- get mykey

# canned workload, prints the stats it produces
cargo run -- --capacity 3 demo

# interactive repl: set, get, del, keys, stats, quit
cargo run -- repl
```

## Tests

```sh
cargo test
```

Covers capacity-bound eviction, recency refresh on get, TTL expiry as a miss plus removal, stats correctness across a known sequence, size never exceeding capacity, and zero-capacity safety.

## Design

See [DESIGN.md](DESIGN.md) for the LRU structure, the TTL model, and the stats accounting.

By Pavan Nallamothu.
