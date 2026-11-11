//! 带容量的 LRU 表：界面层各种「按 key 记住结果」的缓存统一用它，避免只进不出。
//!
//! 接口与 `HashMap` 的常用子集一致（`get` / `insert` / `remove` / `contains_key`），
//! 访问时间戳放在 `Cell` 里，所以 `get` 只需要 `&self`（`RefCell::borrow()` 下也能更新最近使用时间）。
//! 超过容量时淘汰最久未使用的一批（一次淘汰 1/4，摊薄扫描成本）。

use std::borrow::Borrow;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::Hash;

pub struct Lru<K, V> {
    map: HashMap<K, (V, Cell<u64>)>,
    tick: Cell<u64>,
    cap: usize,
    /// 返回 true 的条目不淘汰（如仍被编辑器持有、还有未落盘修改的对象）
    pin: Option<fn(&K, &V) -> bool>,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    pub fn new(cap: usize) -> Self {
        Lru { map: HashMap::new(), tick: Cell::new(0), cap: cap.max(1), pin: None }
    }

    /// 带「钉住」规则的 LRU：`pin` 返回 true 的条目永不淘汰（可能因此暂时超过容量）。
    pub fn with_pin(cap: usize, pin: fn(&K, &V) -> bool) -> Self {
        Lru { pin: Some(pin), ..Self::new(cap) }
    }

    fn stamp(&self) -> u64 {
        let t = self.tick.get() + 1;
        self.tick.set(t);
        t
    }

    pub fn get<Q>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (v, at) = self.map.get(k)?;
        at.set(self.stamp());
        Some(v)
    }

    pub fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let t = self.stamp();
        let (v, at) = self.map.get_mut(k)?;
        at.set(t);
        Some(v)
    }

    #[cfg(test)]
    pub fn contains_key<Q>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(k)
    }

    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        let t = self.stamp();
        let old = self.map.insert(k, (v, Cell::new(t))).map(|x| x.0);
        if self.map.len() > self.cap {
            let pin = self.pin;
            self.evict_where(|k, v| !pin.is_some_and(|p| p(k, v)));
        }
        old
    }

    pub fn remove<Q>(&mut self, k: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.remove(k).map(|x| x.0)
    }

    #[cfg(test)]
    pub fn iter_ids(&self) -> Vec<K> {
        self.map.keys().cloned().collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.map.iter_mut().map(|(k, x)| (k, &mut x.0))
    }

    /// 超过容量时，从最久未使用的开始淘汰 `can_evict` 允许的条目，直到降到容量的 3/4。返回被淘汰的 key。
    pub fn evict_where(&mut self, can_evict: impl Fn(&K, &V) -> bool) -> Vec<K> {
        if self.map.len() <= self.cap {
            return vec![];
        }
        let target = self.cap - self.cap / 4;
        let mut order: Vec<(u64, K)> = self.map.iter().filter(|(k, x)| can_evict(k, &x.0)).map(|(k, x)| (x.1.get(), k.clone())).collect();
        order.sort_by_key(|x| x.0);
        let mut out = vec![];
        for (_, k) in order {
            if self.map.len() <= target {
                break;
            }
            self.map.remove(&k);
            out.push(k);
        }
        out
    }
}

impl<K: Hash + Eq + Clone, V> Default for Lru<K, V> {
    fn default() -> Self {
        Lru::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::Lru;

    #[test]
    fn evicts_least_recently_used() {
        let mut l = Lru::new(4);
        for i in 0..4 {
            l.insert(i, i);
        }
        assert_eq!(l.get(&0), Some(&0)); // 0 变成最近使用
        l.insert(4, 4); // 超容量：淘汰到 3 条，最久未用的 1、2 先走
        assert_eq!(l.len(), 3);
        assert!(l.contains_key(&0) && l.contains_key(&4) && l.contains_key(&3));
    }

    #[test]
    fn respects_pinned_entries() {
        let mut l = Lru::new(2);
        l.insert("a", 1);
        l.insert("b", 2);
        l.map.insert("c", (3, std::cell::Cell::new(99)));
        let gone = l.evict_where(|k, _| *k != "a");
        assert!(l.contains_key("a"), "钉住的条目不淘汰");
        assert_eq!(gone.len(), 1);
    }
}
