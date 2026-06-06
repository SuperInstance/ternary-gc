# ternary-gc

**Garbage collection for GPU-managed memory using ternary marking {-1, 0, +1} — three-level reachability (Reachable, MaybeReachable, Unreachable) with BFS mark phase, sweep phase, and freed-byte tracking.**

## Background

Garbage collection (GC) is the automatic reclamation of memory that is no longer reachable from the program's root set. The classic algorithm is mark-and-sweep: traverse the object graph from roots, mark every reachable object, then sweep (free) everything unmarked. Variants include generational GC (young/old generations), concurrent GC (mark while the mutator runs), and reference counting.

This crate introduces **ternary marking** to garbage collection. Instead of the traditional binary mark (reachable / unreachable), it uses three states:

- **Reachable (+1)**: Directly reachable from a root. This object *must* survive.
- **MaybeReachable (0)**: Transitively reachable — referenced by a reachable object, but not directly rooted. This object *should probably* survive.
- **Unreachable (−1)**: Not reachable from any root. This object *can* be freed.

The three-level scheme provides information that binary marking cannot. In binary GC, a transitive reference looks the same as a direct root reference — both are "reachable." Ternary marking distinguishes them. This is valuable for:

1. **Generational hints**: MaybeReachable objects (transitively reachable) are candidates for demotion to a younger generation — they might become unreachable sooner.
2. **Compaction priority**: Unreachable objects are freed immediately. MaybeReachable objects that survive multiple cycles without becoming directly reachable might be compaction candidates.
3. **Debugging**: If an object is MaybeReachable but expected to be Reachable, there's a missing root registration.

The BFS-based mark phase explores the object graph breadth-first. Root objects are marked Reachable (+1). Their direct references are marked MaybeReachable (0). References of MaybeReachable objects are also marked MaybeReachable (0) — not promoted to Reachable, because they're not directly rooted. The sweep phase frees only Unreachable (−1) objects.

## How It Works

### Core Types

**`Mark`** — Three-level marking enum:
```rust
pub enum Mark {
    Reachable = 1,       // Directly from root
    MaybeReachable = 0,  // Transitively from root
    Unreachable = -1,    // Not reachable
}
```

**`GcObject`** — A heap object:
```rust
pub struct GcObject {
    pub id: u64,
    pub size: usize,
    pub refs: Vec<u64>,       // outgoing references (edges in the object graph)
    pub generation: u32,
}
```

**`TernaryGc`** — The garbage collector:
```rust
pub struct TernaryGc {
    objects: HashMap<u64, GcObject>,
    marks: HashMap<u64, Mark>,
    roots: HashSet<u64>,
    next_id: u64,
    freed_bytes: u64,
    sweep_count: u64,
}
```

### Mark Phase (BFS)

```
1. Reset all marks to Unreachable (-1)
2. For each root:
   - Mark as Reachable (+1)
   - Add to BFS queue
3. While queue is not empty:
   - Dequeue object
   - For each reference:
     - If currently Unreachable:
       - Mark as MaybeReachable (0)
       - Enqueue for further traversal
```

Key property: an object is marked MaybeReachable at most once. If it's already Reachable or MaybeReachable, we skip it. This prevents infinite loops in cyclic graphs.

### Sweep Phase

```
1. Collect all objects marked Unreachable (-1)
2. Remove from objects map
3. Accumulate freed bytes
4. Increment sweep count
```

Only Unreachable objects are freed. Reachable and MaybeReachable objects survive.

### API

```rust
let mut gc = TernaryGc::new();

// Allocate objects
let a = gc.allocate(64, vec![]);   // 64 bytes, no refs
let b = gc.allocate(128, vec![]);  // 128 bytes, no refs

// Create reference: a → b
gc.objects.get_mut(&a).unwrap().refs.push(b);

// Root a (b becomes transitively reachable)
gc.add_root(a);

// Collect garbage
let freed = gc.collect();
// freed = 0 (nothing unreachable)

// Remove root — both a and b become unreachable
gc.remove_root(a);
let freed = gc.collect();
// freed = 64 + 128 = 192 bytes
```

### Design Decisions

1. **HashMap-based object store**: Objects are stored in a `HashMap<u64, GcObject>` keyed by ID. This provides O(1) lookup for the mark phase's graph traversal. Alternative: arena allocation with ID-based indexing.

2. **No MaybeReachable promotion**: Transitively reachable objects stay at MaybeReachable (0) even if referenced by multiple Reachable objects. Promotion to Reachable only happens via `add_root()`. This keeps the semantics clear: Reachable = "I have a root reference."

3. **Freed-byte tracking**: The GC accumulates total freed bytes across all collections (`freed_bytes()`) and counts sweep operations (`sweep_count()`). These metrics are essential for GC tuning and memory leak detection.

## Experimental Results

All **8 tests pass**:

| Test | Result |
|------|--------|
| `test_allocate` | Object created, initial mark = Unreachable (−1) |
| `test_root_mark_reachable` | Rooted object marked Reachable (+1) |
| `test_transitive_maybe` | Root → A → B: A is Reachable, B is MaybeReachable |
| `test_sweep_unreachable` | 2 unreachable objects (256B) freed; 1 rooted (64B) kept |
| `test_full_collect` | 2 unrooted (300B total) freed; 1 rooted (50B) kept |
| `test_freed_tracking` | After one collection: freed_bytes = 100 |
| `test_remove_root` | After root removal + mark: object becomes Unreachable |
| `test_chain_reachable` | A→B→C chain: all transitively marked MaybeReachable |

Key findings:
- **Chain reachability**: In A→B→C with A rooted, all three survive: A is Reachable (+1), B is MaybeReachable (0), C is MaybeReachable (0). The BFS correctly propagates through the chain.
- **Root removal**: After `remove_root(a)` + `mark()`, all objects in the graph become Unreachable (−1). The next `sweep()` frees them all.
- **Freed bytes**: The GC accurately tracks total memory reclaimed: `freed_bytes()` returns the cumulative sum across all collections.

## Impact

The ternary marking scheme {-1, 0, +1} is what distinguishes this GC from standard mark-and-sweep. In a binary GC, the sweep phase treats all surviving objects identically — they're "live." In ternary GC, the MaybeReachable (0) objects are *conditionally live* — they survive this cycle but are flagged as potential garbage in future cycles. This creates a natural foundation for generational GC without requiring separate generation spaces.

The zero state (MaybeReachable) also serves as a *warning level*. In monitoring dashboards, you can track the ratio of MaybeReachable to Reachable objects. A high ratio means many objects are surviving only through transitive references — they're likely to become garbage soon. This is a predictor of GC pressure.

## Use Cases

1. **GPU memory management** — Manage GPU-side object graphs where allocation/deallocation is expensive; ternary marking provides generation hints for bulk deallocation
2. **Game engine entity systems** — Track entity references in game worlds; MaybeReachable entities are candidates for deferred destruction
3. **Agent-based simulation** — Manage agent populations where agents reference each other; ternary marking identifies agents that are alive but not directly accessible
4. **Document object models** — Manage DOM-like trees where nodes reference parents and children; MaybeReachable nodes might be orphaned subtrees
5. **Cyclic reference detection** — The BFS mark phase naturally handles cycles; objects in reference cycles that aren't rooted are correctly identified as Unreachable

## Open Questions

1. **MaybeReachable promotion policy**: Should objects that have been MaybeReachable for N consecutive collections be promoted to Reachable (they're clearly needed) or demoted to Unreachable (they're never getting rooted)? This is a tuning parameter that depends on workload.
2. **Concurrent marking**: The current implementation is stop-the-world (mutator pauses during mark+sweep). Can the ternary marking scheme support concurrent marking where MaybeReachable is a "maybe" state during collection?
3. **Compaction integration**: After sweep, should MaybeReachable objects be compacted into a contiguous region? This would improve cache locality but requires updating all references.

## Connection to Oxide Stack

`ternary-gc` sits at the **cudaclaw** layer as the memory management subsystem for persistent GPU kernels. GPU kernels that run indefinitely (persistent kernels) need garbage collection for dynamic data structures. The ternary marking scheme integrates with warp-consensus: each warp can independently mark its local objects, and the MaybeReachable state represents "reachable from another warp" — a natural boundary for distributed GC.

At the **flux-core** layer, the A2A agent protocol creates dynamic agent graphs where agents reference each other. The ternary GC manages this graph, with MaybeReachable agents being those that are referenced but not directly accessible — candidates for hibernation or migration.

## Stats

| Metric | Value |
|--------|-------|
| Lines of Rust | ~120 |
| Test count | 8 |
| Public types | 3 (Mark, GcObject, TernaryGc) |
| Public functions | 9 |
| Dependencies | 0 |

## Install

```toml
[dependencies]
ternary-gc = "0.1.0"
```

## License

MIT
