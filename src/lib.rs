//! # ternary-gc
//!
//! Garbage collection for GPU memory with ternary marking.

use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark { Reachable = 1, MaybeReachable = 0, Unreachable = -1 }

#[derive(Debug, Clone)]
pub struct GcObject {
    pub id: u64,
    pub size: usize,
    pub refs: Vec<u64>,
    pub generation: u32,
}

pub struct TernaryGc {
    objects: HashMap<u64, GcObject>,
    marks: HashMap<u64, Mark>,
    roots: HashSet<u64>,
    next_id: u64,
    freed_bytes: u64,
    sweep_count: u64,
}

impl TernaryGc {
    pub fn new() -> Self {
        Self { objects: HashMap::new(), marks: HashMap::new(), roots: HashSet::new(), next_id: 1, freed_bytes: 0, sweep_count: 0 }
    }

    pub fn allocate(&mut self, size: usize, refs: Vec<u64>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, GcObject { id, size, refs, generation: 0 });
        self.marks.insert(id, Mark::Unreachable);
        id
    }

    pub fn add_root(&mut self, id: u64) { self.roots.insert(id); }
    pub fn remove_root(&mut self, id: u64) { self.roots.remove(&id); }

    /// Mark phase: BFS from roots. Reachable=+1, transitive but not direct=0, else=-1.
    pub fn mark(&mut self) {
        // Reset all to unreachable
        for mark in self.marks.values_mut() { *mark = Mark::Unreachable; }

        let mut queue: VecDeque<u64> = VecDeque::new();
        for &root in &self.roots {
            if self.objects.contains_key(&root) {
                self.marks.insert(root, Mark::Reachable);
                queue.push_back(root);
            }
        }

        while let Some(id) = queue.pop_front() {
            if let Some(obj) = self.objects.get(&id) {
                for &ref_id in &obj.refs {
                    if let Some(mark) = self.marks.get_mut(&ref_id) {
                        if *mark == Mark::Unreachable {
                            *mark = Mark::MaybeReachable;
                            queue.push_back(ref_id);
                        }
                    }
                }
            }
        }
    }

    /// Sweep phase: free unreachable objects.
    pub fn sweep(&mut self) -> usize {
        let to_remove: Vec<u64> = self.marks.iter()
            .filter(|(_, &m)| m == Mark::Unreachable)
            .map(|(&id, _)| id)
            .collect();

        let mut freed = 0;
        for id in &to_remove {
            if let Some(obj) = self.objects.remove(id) {
                freed += obj.size;
            }
            self.marks.remove(id);
        }
        self.freed_bytes += freed as u64;
        self.sweep_count += 1;
        freed
    }

    /// Full GC cycle: mark then sweep.
    pub fn collect(&mut self) -> usize { self.mark(); self.sweep() }

    pub fn mark_of(&self, id: u64) -> Option<Mark> { self.marks.get(&id).copied() }
    pub fn object_count(&self) -> usize { self.objects.len() }
    pub fn total_size(&self) -> usize { self.objects.values().map(|o| o.size).sum() }
    pub fn freed_bytes(&self) -> u64 { self.freed_bytes }
    pub fn sweep_count(&self) -> u64 { self.sweep_count }
}

impl Default for TernaryGc { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate() {
        let mut gc = TernaryGc::new();
        let id = gc.allocate(128, vec![]);
        assert_eq!(gc.object_count(), 1);
        assert_eq!(gc.mark_of(id), Some(Mark::Unreachable));
    }

    #[test]
    fn test_root_mark_reachable() {
        let mut gc = TernaryGc::new();
        let id = gc.allocate(64, vec![]);
        gc.add_root(id);
        gc.mark();
        assert_eq!(gc.mark_of(id), Some(Mark::Reachable));
    }

    #[test]
    fn test_transitive_maybe() {
        let mut gc = TernaryGc::new();
        let a = gc.allocate(64, vec![]);
        let b = gc.allocate(64, vec![]);
        gc.add_root(a);
        // Manually set a -> b ref
        gc.objects.get_mut(&a).unwrap().refs.push(b);
        gc.mark();
        assert_eq!(gc.mark_of(a), Some(Mark::Reachable));
        assert_eq!(gc.mark_of(b), Some(Mark::MaybeReachable));
    }

    #[test]
    fn test_sweep_unreachable() {
        let mut gc = TernaryGc::new();
        gc.allocate(256, vec![]); // unreachable
        let kept = gc.allocate(64, vec![]);
        gc.add_root(kept);
        gc.mark();
        let freed = gc.sweep();
        assert_eq!(freed, 256);
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_full_collect() {
        let mut gc = TernaryGc::new();
        gc.allocate(100, vec![]);
        gc.allocate(200, vec![]);
        let root = gc.allocate(50, vec![]);
        gc.add_root(root);
        let freed = gc.collect();
        assert_eq!(freed, 300);
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_freed_tracking() {
        let mut gc = TernaryGc::new();
        gc.allocate(100, vec![]);
        gc.collect();
        assert_eq!(gc.freed_bytes(), 100);
    }

    #[test]
    fn test_remove_root() {
        let mut gc = TernaryGc::new();
        let id = gc.allocate(64, vec![]);
        gc.add_root(id);
        gc.remove_root(id);
        gc.mark();
        assert_eq!(gc.mark_of(id), Some(Mark::Unreachable));
    }

    #[test]
    fn test_chain_reachable() {
        let mut gc = TernaryGc::new();
        let a = gc.allocate(32, vec![]);
        let b = gc.allocate(32, vec![]);
        let c = gc.allocate(32, vec![]);
        gc.objects.get_mut(&a).unwrap().refs.push(b);
        gc.objects.get_mut(&b).unwrap().refs.push(c);
        gc.add_root(a);
        gc.mark();
        assert_eq!(gc.mark_of(a), Some(Mark::Reachable));
        assert_eq!(gc.mark_of(b), Some(Mark::MaybeReachable));
        assert_eq!(gc.mark_of(c), Some(Mark::MaybeReachable));
    }
}
