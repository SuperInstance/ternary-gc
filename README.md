# ternary-gc

Garbage collection for GPU memory with ternary reachability marking.

## Why This Exists

Standard GC uses binary reachability: alive or dead. GPU memory allocators need more nuance. An object might be directly reachable from a root (definitely alive), transitively reachable through another object (probably alive but check again soon), or truly unreachable (safe to free). Ternary GC captures this three-state distinction directly in the mark phase, giving you a "maybe reachable" cohort that's cheap to re-verify rather than conservatively keeping or aggressively freeing.

This is especially useful for GPU workloads where allocation sizes are large and false positives (keeping garbage) waste significant VRAM, but false negatives (freeing live data) cause kernel faults.

## Architecture

### Core Types

- **`Mark`** — The ternary enum: `Reachable (+1)`, `MaybeReachable (0)`, `Unreachable (-1)`.
- **`GcObject`** — A tracked allocation with `id`, `size` in bytes, `refs` (outgoing edges), and `generation`.
- **`TernaryGc`** — The collector itself. Holds an object graph, mark map, and root set.

### How It Works

1. **Allocate** objects with `allocate(size, refs)` — they start as `Unreachable`.
2. Register roots with `add_root(id)`.
3. **Mark phase** (`mark()`): BFS from roots. Direct roots get `Reachable`. Anything discovered via following refs gets `MaybeReachable`. Everything else stays `Unreachable`.
4. **Sweep phase** (`sweep()`): Free all `Unreachable` objects, return bytes reclaimed.
5. `collect()` runs mark-then-sweep as one operation.

## Usage

```rust
use ternary_gc::TernaryGc;

let mut gc = TernaryGc::new();

// Allocate some GPU buffers
let a = gc.allocate(1024, vec![]);  // 1KB, no refs
let b = gc.allocate(2048, vec![]);  // 2KB, no refs
let c = gc.allocate(512, vec![b]);  // 512B, refs to b

gc.add_root(a);

// Run collection
let freed = gc.collect();
println!("Reclaimed {} bytes", freed);

// Check individual marks
use ternary_gc::Mark;
assert_eq!(gc.mark_of(a), Some(Mark::Reachable));  // root
```

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `new()` | `TernaryGc` | Create a new collector |
| `allocate(size, refs)` | `u64` | Allocate an object, returns its ID |
| `add_root(id)` | `()` | Register a root object |
| `remove_root(id)` | `()` | Unregister a root |
| `mark()` | `()` | Run the ternary mark phase |
| `sweep()` | `usize` | Free unreachable objects, return bytes freed |
| `collect()` | `usize` | Full mark + sweep cycle |
| `mark_of(id)` | `Option<Mark>` | Check an object's mark state |
| `object_count()` | `usize` | Live object count |
| `total_size()` | `usize` | Total live bytes |
| `freed_bytes()` | `u64` | Cumulative bytes freed across all sweeps |
| `sweep_count()` | `u64` | Number of sweep phases executed |

## The Deeper Idea

The "maybe reachable" state is a form of **generational hints without generations**. In a traditional generational GC, you'd need write barriers and remembered sets. Here, the BFS mark naturally separates direct-from-root (strong) from transitive (weak) without any barrier overhead. On GPU where memory access patterns are predictable (kernels hold references in predictable DAGs), this gives you a lightweight way to prioritize what to keep vs. what to verify next cycle.

## Related Crates

- **ternary-accumulator** — ternary gradient accumulation for training
- **ternary-cache** — caching with ternary freshness states
- **ternary-retry** — retry logic with ternary outcomes (Success / Retryable / PermanentFail)
