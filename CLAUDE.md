# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build
cargo build --features dynamic_mem   # Vec-based heap instead of fixed arrays

# Run
cargo run --release                  # Silent mode
cargo run --release -- help          # Show available debug flags
cargo run --release -- gc            # Show GC stats after each collection
cargo run --release -- heap          # Show heap visualization after GC
cargo run --release -- freelist      # Show free list after each GC
cargo run --release -- allocates     # Show all allocations

# Lint
cargo clippy

# Test (no unit test suite; main.rs is the test harness)
cargo run --release
```

## Architecture

Rustgc is a slot-based mark-and-sweep memory allocator/GC demo, intended as a foundation for building language runtimes.

### Memory model

The heap is a flat array of `usize` slots. Objects are identified by their starting slot index (not pointers). Each object has a 1-slot size header followed by indexable data slots.

Two heap configurations via feature flags:
- **Default**: Fixed 128K-slot array
- `dynamic_mem`: `Vec`-based (growable)

### Core data structures (`src/gc/mod.rs`)

- `Memory` struct holds: the heap (`mem`), a free list (`head`), mark bits (128-bit words), a roots `Vec`, and stats
- **Free list**: singly-linked list threaded through the first slot of each free block
- **Mark bits**: one bit per slot, packed into `u128` words for fast bulk clearing
- **Roots**: explicit root set that GC starts tracing from

### GC algorithm

1. **Mark** (`mark_and_scan`): recursively marks all slots reachable from roots
2. **Sweep**: walks the heap linearly, rebuilds the free list from unmarked blocks, coalesces adjacent free blocks
3. GC triggers automatically when allocation fails

### Main test harness (`src/main.rs`)

Creates an object graph (root with 8 slots, each pointing to sub-objects of random size 1–32 slots), then runs 500,000 cycles of random replacement and reallocation to exercise the GC. Verifies object invariants after each GC.

### Module layout

- `src/main.rs` — test harness and debug flag parsing
- `src/gc/mod.rs` — allocator, GC, and all core data structures
- `src/rnd/mod.rs` — simple RNG for generating varied allocation sizes
