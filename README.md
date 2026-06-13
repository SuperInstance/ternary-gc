# ternary-gc

Garbage collection for GPU memory using ternary mark-sweep. Each object receives a ternary mark — **{+1 = reachable, 0 = maybe-reachable, −1 = unreachable}** — enabling three-tier collection precision instead of the binary reachable/unreachable split used by classical GC.

## Why It Matters

Standard mark-sweep GC (McCarthy, 1960) uses binary marks: an object is either reachable or not. This forces a conservative approach — any object with a *possible* reference must be kept, leading to retention of garbage that *might* be reachable.

Ternary marking adds a **third state** for objects that are transitively reachable but not directly rooted. This distinction enables:

- **Generational priority**: Directly-rooted objects (+1) are more valuable than transitively-reached ones (0)
- **Incremental reclamation**: Maybe-reachable objects (0) can be demoted and reclaimed under memory pressure before becoming fully unreachable
- **Reduced pause times**: Three-tier marking allows the sweep phase to be selective — reclaim only −1 objects in fast cycles, include 0 objects in full cycles
- **GPU memory aware**: Designed for GPU object graphs where allocation/deallocation is expensive (~10–100 μs per cudaFree)

## How It Works

### Ternary Mark Phase

The mark phase performs a BFS from root set R:

```
1. For each root r ∈ R:       mark(r) ← Reachable (+1)
2. BFS from R:
   For each object o reachable from R:
     If mark(o) == Unreachable (−1):
       mark(o) ← MaybeReachable (0)
3. All unvisited objects remain Unreachable (−1)
```

The key distinction: roots and their direct descendants get +1 (Reachable), while objects discovered through transitive reference chains get 0 (MaybeReachable). This is a **depth-of-reach** signal.

### Sweep Phase

Objects marked −1 (Unreachable) are freed:

```
For each object o where mark(o) == −1:
    deallocate(o)
    freed_bytes += size(o)
```

Under memory pressure, the collector can also reclaim MaybeReachable (0) objects, trading potential retention for immediate memory recovery.

### Mark Transitions

```
Unreachable (−1)  ──BFS visit──►  MaybeReachable (0)
MaybeReachable (0) ──is root──►   Reachable (+1)
Reachable (+1)    ──GC reset──►   Unreachable (−1)  [start of each cycle]
```

### Object Graph

Each `GcObject` has:
- `id`: Unique 64-bit identifier
- `size`: Byte footprint for accounting
- `refs`: List of child object IDs (the reference graph edges)
- `generation`: For generational collection strategies

### Complexity

| Operation | Time | Space |
|-----------|------|-------|
| `allocate(size, refs)` | O(1) amortized | O(1) |
| `add_root(id) / remove_root(id)` | O(1) | O(1) |
| `mark()` | O(V + E) | O(V) |
| `sweep()` | O(V) | O(V) |
| `collect()` = mark + sweep | O(V + E) | O(V) |

Where V = number of objects, E = total reference edges. This matches the classical mark-sweep bound (Jones & Lins, 1996).

### Comparison with Binary Mark-Sweep

| Feature | Binary GC | Ternary GC (this crate) |
|---------|-----------|------------------------|
| Mark states | 2 | 3 |
| Reachability depth | None | Direct vs. transitive |
| Selective sweep | No | Yes (sweep only −1, or −1 + 0) |
| Generational hint | By age only | By reachability depth + age |
| Memory pressure response | Full collection | Gradual (−1 first, then 0) |

## Quick Start

```rust
use ternary_gc::{TernaryGc, Mark};

let mut gc = TernaryGc::new();

// Allocate objects with references
let tensor_a = gc.allocate(1024, vec![]);
let tensor_b = gc.allocate(2048, vec![tensor_a]);
let orphan   = gc.allocate(512,  vec![]);  // no roots → will be collected

// Root the live objects
gc.add_root(tensor_b);  // tensor_b → tensor_a transitively reachable

// Run GC
let freed = gc.collect();
println!("Freed {} bytes", freed);  // 512 (the orphan)

assert_eq!(gc.mark_of(tensor_a), Some(Mark::MaybeReachable)); // transitive
assert_eq!(gc.mark_of(tensor_b), Some(Mark::Reachable));      // rooted
assert_eq!(gc.mark_of(orphan),   None);                       // freed
```

## API

### `TernaryGc`

| Method | Description |
|--------|-------------|
| `new()` | Create empty collector |
| `allocate(size, refs) -> u64` | Allocate object with references |
| `add_root(id) / remove_root(id)` | Manage root set |
| `mark()` | Run ternary mark phase (BFS from roots) |
| `sweep() -> usize` | Free unreachable objects, return bytes freed |
| `collect() -> usize` | Full mark + sweep cycle |
| `mark_of(id) -> Option<Mark>` | Query current mark |
| `object_count() / total_size()` | Statistics |
| `freed_bytes() / sweep_count()` | Lifetime accounting |

### `Mark`

```rust
pub enum Mark {
    Reachable = 1,       // Directly rooted
    MaybeReachable = 0,  // Transitively discovered
    Unreachable = -1,    // Not visited this cycle
}
```

## Architecture Notes

This crate implements the **γ (gamma) memory management layer** of the γ + η = C framework:

- **γ (gamma)**: Resource lifecycle management — deciding what to keep and what to free. This crate provides γ-level garbage collection for GPU memory.
- **η (eta)**: The compute layer that allocates and uses GPU objects. Ecosystem inference and simulation crates act as the η-layer mutator.
- **C**: The complete memory-safe GPU compute system. γ ensures η never accesses freed memory.

The ternary marks {+1, 0, −1} directly parallel the ternary weight domain used throughout the ecosystem, making the GC's decisions expressible in the same algebraic framework.

## References

- **Mark-Sweep GC**: McCarthy, J., "Recursive Functions of Symbolic Expressions and Their Computation by Machine, Part I," CACM, 3(4), 184-195, 1960.
- **GC Theory**: Jones, R. & Lins, R., "Garbage Collection: Algorithms for Automatic Dynamic Memory Management," Wiley, 1996.
- **Generational GC**: Ungar, D., "Generation Scavenging: A Non-disruptive High Performance Storage Reclamation Algorithm," ACM SIGPLAN, 1984.
- **CUDA Memory Management**: NVIDIA, "CUDA C++ Programming Guide," Memory Management chapter, 2024.
- **Tri-color Marking**: Dijkstra, E.W. et al., "On-the-Fly Garbage Collection: An Exercise in Cooperation," CACM, 21(11), 966-975, 1978. The ternary scheme extends tri-color marking with a quantitative reachability metric.

## License

MIT
