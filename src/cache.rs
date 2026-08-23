use crate::clock::Clock;
use std::collections::HashMap;

/// A single slot in the intrusive LRU list. Slots live in a flat `Vec` so
/// that unlinking and relinking are index swaps, not allocations.
struct Node<V> {
    key: String,
    value: V,
    expires_at: Option<u64>,
    prev: Option<usize>,
    next: Option<usize>,
}

/// Hit/miss/eviction/expiration counters plus the live size.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub size: usize,
}

impl Stats {
    /// hits / (hits + misses), 0.0 when nothing has been requested yet.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// A capacity-bounded, TTL-aware, LRU cache. Keys are `String`, values are
/// whatever `V` the caller wants (a `String`, raw bytes, anything `Clone`).
///
/// Backed by a `HashMap<String, usize>` for O(1) lookup plus a slab of
/// `Node`s wired into a doubly linked list for O(1) recency updates and
/// O(1) eviction of the least-recently-used entry. A free list lets removed
/// slots be reused instead of leaking Vec space.
pub struct Cache<V, C: Clock> {
    capacity: usize,
    map: HashMap<String, usize>,
    slots: Vec<Option<Node<V>>>,
    free: Vec<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    clock: C,
    stats: Stats,
}

impl<V: Clone, C: Clock> Cache<V, C> {
    /// `capacity` of 0 is legal: the cache simply never retains anything.
    pub fn new(capacity: usize, clock: C) -> Self {
        Cache {
            capacity,
            map: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            head: None,
            tail: None,
            clock,
            stats: Stats::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn stats(&self) -> Stats {
        let mut s = self.stats;
        s.size = self.map.len();
        s
    }

    /// Keys most-recently-used first. Used by the CLI and by anything that
    /// wants to see the eviction order without mutating recency.
    pub fn keys_by_recency(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.map.len());
        let mut cur = self.head;
        while let Some(idx) = cur {
            let node = self.slots[idx].as_ref().expect("linked slot is live");
            out.push(node.key.clone());
            cur = node.next;
        }
        out
    }

    /// Insert or overwrite `key`. `ttl_millis` of `None` means "never expires".
    /// If the cache is at capacity and `key` is new, the least-recently-used
    /// entry is evicted first. Never panics, never exceeds `capacity`.
    pub fn put(&mut self, key: String, value: V, ttl_millis: Option<u64>) {
        if self.capacity == 0 {
            return;
        }
        let now = self.clock.now_millis();
        let expires_at = ttl_millis.map(|d| now.saturating_add(d));

        if let Some(&idx) = self.map.get(&key) {
            {
                let node = self.slots[idx].as_mut().expect("mapped slot is live");
                node.value = value;
                node.expires_at = expires_at;
            }
            self.touch(idx);
            return;
        }

        if self.map.len() >= self.capacity {
            self.evict_lru();
        }

        let idx = self.alloc_slot(Node {
            key: key.clone(),
            value,
            expires_at,
            prev: None,
            next: None,
        });
        self.map.insert(key, idx);
        self.push_front(idx);
    }

    /// Look up `key`. Expired entries are removed and counted as a miss and
    /// an expiration. A live hit refreshes recency. Returns a clone of the
    /// value so the cache keeps ownership.
    pub fn get(&mut self, key: &str) -> Option<V> {
        let idx = match self.map.get(key) {
            Some(&idx) => idx,
            None => {
                self.stats.misses += 1;
                return None;
            }
        };

        let now = self.clock.now_millis();
        let expired = self.slots[idx]
            .as_ref()
            .expect("mapped slot is live")
            .expires_at
            .map_or(false, |exp| now >= exp);

        if expired {
            self.remove_slot(idx);
            self.stats.misses += 1;
            self.stats.expirations += 1;
            return None;
        }

        self.touch(idx);
        self.stats.hits += 1;
        Some(self.slots[idx].as_ref().expect("mapped slot is live").value.clone())
    }

    /// Remove `key` outright, regardless of expiry. Not counted as a miss
    /// or an expiration since it was not a lookup.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.map.get(key).copied() {
            Some(idx) => {
                self.remove_slot(idx);
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.slots.clear();
        self.free.clear();
        self.head = None;
        self.tail = None;
    }

    // -- internal list/slab plumbing --------------------------------------

    fn alloc_slot(&mut self, node: Node<V>) -> usize {
        if let Some(idx) = self.free.pop() {
            self.slots[idx] = Some(node);
            idx
        } else {
            self.slots.push(Some(node));
            self.slots.len() - 1
        }
    }

    fn unlink(&mut self, idx: usize) {
        let (prev, next) = {
            let node = self.slots[idx].as_ref().expect("live slot");
            (node.prev, node.next)
        };
        match prev {
            Some(p) => self.slots[p].as_mut().expect("live slot").next = next,
            None => self.head = next,
        }
        match next {
            Some(n) => self.slots[n].as_mut().expect("live slot").prev = prev,
            None => self.tail = prev,
        }
    }

    fn push_front(&mut self, idx: usize) {
        {
            let node = self.slots[idx].as_mut().expect("live slot");
            node.prev = None;
            node.next = self.head;
        }
        if let Some(h) = self.head {
            self.slots[h].as_mut().expect("live slot").prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    fn touch(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.push_front(idx);
    }

    fn remove_slot(&mut self, idx: usize) {
        self.unlink(idx);
        let node = self.slots[idx].take().expect("live slot");
        self.map.remove(&node.key);
        self.free.push(idx);
    }

    fn evict_lru(&mut self) {
        if let Some(idx) = self.tail {
            self.remove_slot(idx);
            self.stats.evictions += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn cache(capacity: usize) -> Cache<String, TestClock> {
        Cache::new(capacity, TestClock::new(0))
    }

    #[test]
    fn eviction_removes_least_recently_used() {
        let mut c = cache(2);
        c.put("a".into(), "1".into(), None);
        c.put("b".into(), "2".into(), None);
        // a is LRU, b is MRU. Inserting c should evict a.
        c.put("c".into(), "3".into(), None);

        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("b"), Some("2".to_string()));
        assert_eq!(c.get("c"), Some("3".to_string()));
        assert_eq!(c.len(), 2);
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn get_refreshes_recency_so_the_touched_key_survives() {
        let mut c = cache(2);
        c.put("a".into(), "1".into(), None);
        c.put("b".into(), "2".into(), None);
        // touch a, making b the LRU
        assert_eq!(c.get("a"), Some("1".to_string()));
        c.put("c".into(), "3".into(), None);

        assert_eq!(c.get("b"), None, "b should have been evicted, not a");
        assert_eq!(c.get("a"), Some("1".to_string()));
        assert_eq!(c.get("c"), Some("3".to_string()));
    }

    #[test]
    fn ttl_expired_entry_is_a_miss_and_is_removed() {
        use std::rc::Rc;

        struct SharedClock(Rc<TestClock>);
        impl Clock for SharedClock {
            fn now_millis(&self) -> u64 {
                self.0.now_millis()
            }
        }

        let inner = Rc::new(TestClock::new(0));
        let mut c: Cache<String, SharedClock> = Cache::new(10, SharedClock(inner.clone()));

        c.put("a".into(), "1".into(), Some(1_000));
        assert_eq!(c.get("a"), Some("1".to_string()));

        inner.advance(1_000);
        assert_eq!(c.get("a"), None, "entry should be expired and evicted");
        assert_eq!(c.len(), 0, "expired entry must be removed, not just hidden");

        let stats = c.stats();
        assert_eq!(stats.expirations, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn stats_are_correct_after_a_known_sequence() {
        let mut c = cache(2);
        c.put("a".into(), "1".into(), None);
        c.put("b".into(), "2".into(), None);

        assert_eq!(c.get("a"), Some("1".to_string())); // hit
        assert_eq!(c.get("z"), None); // miss

        c.put("c".into(), "3".into(), None); // evicts b (a was touched, is MRU)

        assert_eq!(c.get("b"), None); // miss
        assert_eq!(c.get("c"), Some("3".to_string())); // hit

        let stats = c.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.evictions, 1);
        assert_eq!(stats.size, 2);
    }

    #[test]
    fn size_never_exceeds_capacity() {
        let mut c = cache(3);
        for i in 0..50 {
            c.put(format!("k{i}"), format!("v{i}"), None);
            assert!(c.len() <= c.capacity());
        }
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn zero_capacity_never_panics_and_never_stores() {
        let mut c = cache(0);
        c.put("a".into(), "1".into(), None);
        assert_eq!(c.get("a"), None);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn overwriting_an_existing_key_does_not_grow_size() {
        let mut c = cache(2);
        c.put("a".into(), "1".into(), None);
        c.put("a".into(), "2".into(), None);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("a"), Some("2".to_string()));
    }

    #[test]
    fn remove_deletes_regardless_of_ttl() {
        let mut c = cache(2);
        c.put("a".into(), "1".into(), None);
        assert!(c.remove("a"));
        assert!(!c.remove("a"));
        assert_eq!(c.get("a"), None);
    }
}
