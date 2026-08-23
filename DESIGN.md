# Design

## Overview

Flashvault is `Cache<V, C: Clock>`, a `String`-keyed cache generic over a cloneable value type `V` and an injected `Clock`. It is built from two pieces working together: a `HashMap<String, usize>` for lookup, and a slab of nodes wired into a doubly linked list for order.

## The LRU structure

Entries live in `slots: Vec<Option<Node<V>>>`. Each `Node` holds the key, the value, an optional expiry timestamp, and `prev`/`next` indices into the same `Vec`. This is an intrusive linked list, the pointers live inside the nodes rather than in a separate structure, so there is no second allocation per entry.

`head` points at the most-recently-used node, `tail` at the least-recently-used. Three operations do all the work:

- `unlink(idx)`: splice a node out, patching its neighbors' `prev`/`next` (or `head`/`tail` if it was at an end).
- `push_front(idx)`: splice a node in at `head`.
- `touch(idx)`: `unlink` then `push_front`, used on every successful `get` and on `put` of an existing key.

A `free: Vec<usize>` list holds indices of removed slots so they get reused on the next insert instead of leaving `Vec` holes or forcing a shift. `map: HashMap<String, usize>` maps each live key to its slot index, so both `get` and `put` are a hash lookup plus a handful of pointer swaps, O(1) regardless of cache size.

## Capacity

`put` checks `map.len() >= capacity` before inserting a genuinely new key. If true, it evicts `tail` first (`evict_lru`), which removes the node, deletes the map entry, and counts the eviction, before the new entry is added. Overwriting an existing key never touches capacity since no new slot is allocated. Capacity zero is legal and simply never retains anything, `put` returns immediately. This is checked directly by `size_never_exceeds_capacity`, which inserts fifty keys into a capacity-3 cache and asserts `len() <= capacity()` after every single one.

## TTL expiry

Expiry is lazy, checked on read rather than swept in the background. Each `put` with a `ttl_millis` records `expires_at = now + ttl_millis` using the injected clock. On `get`, if `now >= expires_at`, the entry is unlinked, removed from the map, counted as both a miss and an expiration, and `None` is returned, the caller never sees a stale value. An entry with no TTL (`ttl_millis: None`) never expires.

Because expiry depends on wall-clock time, tests use a `Clock` trait instead of `std::time::SystemTime` directly. `SystemClock` is the real implementation; `TestClock` holds a `Cell<u64>` that can be set or advanced by hand, so a test can insert an entry, jump the clock forward past its TTL, and assert the miss deterministically, with no `sleep` and no flakiness.

## Stats

`Stats` tracks `hits`, `misses`, `evictions`, `expirations`, and `size` (the last one derived from `map.len()` at read time rather than tracked separately, so it can never drift out of sync). `hit_rate()` is `hits / (hits + misses)`, `0.0` when nothing has been requested yet, avoiding a division by zero. Every counter increments at exactly one place in the code: `hits` and `misses` only inside `get`, `evictions` only inside `evict_lru`, `expirations` only on the lazy-expiry path inside `get`. `stats_are_correct_after_a_known_sequence` walks a fixed sequence of puts and gets and asserts the exact resulting counts.

## No panics

`slots[idx]` indexing inside the linked-list plumbing always operates on indices that came from `map` or from list traversal, both of which only ever hold indices of live, `Some` slots, so the `expect()` calls documenting that invariant should never fire. The zero-capacity path is checked at the top of `put` before any indexing happens, and `get`/`remove` on an absent key simply route through the `map.get` `None` branch. `zero_capacity_never_panics_and_never_stores` and the eviction tests cover the edges directly.
