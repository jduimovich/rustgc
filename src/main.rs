mod gc;
mod rnd;
use std::io::stdout;
use std::io::Write;

use std::env;

const LOOP: usize = 100000;
const MAX_OBJECT_SIZE: usize = 32;
const NUMBER_OF_ROOTS: usize = 128;

fn main() {

  let args: Vec<String> = env::args().collect();

  println!( "Before Init");
  let mut mem = gc::Memory::initialze_memory();
  println!( "New Heap Size {}", mem.total_heap_slots());
  println!( "New Heap  {} objects", mem.live_objects().count());
  for obj in mem.live_objects()  {
    println!( "New Heap, iterate over objects: {}", obj);
  }

  for i in 0..args.len() {
    if args[i] == "allocates" {
      mem.enable_show_allocates(true);
    }
    if args[i] == "abandons" {
      mem.enable_show_abandons(true);
    }
    if args[i] == "freelist" {
      mem.enable_show_freelist(true);
    }
    if args[i] == "heap" {
      mem.enable_show_heap_map(true);
    }
    if args[i] == "gc" {
      mem.enable_show_gc(true);
    }
    if args[i] == "compact" {
      mem.enable_show_compact(true);
    }
    if args[i] == "help" {
      println!(
        "GC Demo Options:
        \t allocates (default off) - show all allocates\n
        \t freelist (default off) - show freelist every gc \n
        \t heap (default off) - show heap every gc\n
        \t gc (default off) - summary  every gc\n
        \t compact (default off) - verbose compact tracing\n
        "
      );
      return;
    }
  }
  let root = mem.allocate_object(NUMBER_OF_ROOTS);
  let root_idx = mem.add_root(root);
  // fill in more objects off this single root
  for i in 0..mem.element_size(mem.get_root(root_idx)) {
    let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
    let root = mem.get_root(root_idx);
    mem.at_put(root, i, allocated);
  }

  let locals = mem.allocate_object(16);
  let root = mem.get_root(root_idx);
  mem.at_put(locals, 0, root);
  mem.at_put(locals, 1, locals); // self reference to test GC

  let mut count = 0;
  while count < LOOP {
    let root = mem.get_root(root_idx);
    for i in 0..mem.element_size(root) {
      let root = mem.get_root(root_idx);
      if i == rnd::rnd_sz(mem.element_size(root)) {
        let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
        let root = mem.get_root(root_idx);
        mem.at_put(root, i, allocated);
      } else {
        let root = mem.get_root(root_idx);
        let mut myobj = mem.at(root, i);
        for j in 0..mem.element_size(myobj) {
          let prev = mem.at(myobj, j);
          if prev != 0 {
            if mem.at(prev, 0) != prev {
              panic!("object should point to self");
            }
            if mem.at(prev, 1) != myobj {
              panic!("object should point to outerobj");
            }
            if mem.element_size(mem.at(prev, 2)) < 2 {
              print!("size is {}\n", mem.element_size(mem.at(prev, 2)));
              panic!("object should be at leat 2 elements");
            }
            if mem.element_size(mem.at(prev, 3)) < 3 {
              print!("size is {}\n", mem.element_size(mem.at(prev, 2)));
              panic!("object should be at leat 2 elements");
            }
          }
          let myobj_idx = mem.add_root(myobj);
          let slot = mem.allocate_object(4); // fill with small objects
          let _root = mem.get_root(root_idx);
          myobj = mem.get_root(myobj_idx);
          mem.remove_root_at(myobj_idx);

          mem.at_put(myobj, j, slot);
          mem.at_put(slot, 0, slot);
          mem.at_put(slot, 1, myobj);

          let myobj_idx = mem.add_root(myobj);
          let slot_idx  = mem.add_root(slot);
          let fill = mem.allocate_object(2);
          let _root = mem.get_root(root_idx);
          let mut slot = mem.get_root(slot_idx);
          myobj = mem.get_root(myobj_idx);
          mem.remove_root_at(slot_idx);
          mem.remove_root_at(myobj_idx);

          mem.at_put(slot, 2, fill);

          let myobj_idx = mem.add_root(myobj);
          let slot_idx  = mem.add_root(slot);
          let fill = mem.allocate_object(3);
          let _root = mem.get_root(root_idx);
          slot = mem.get_root(slot_idx);
          myobj = mem.get_root(myobj_idx);
          mem.remove_root_at(slot_idx);
          mem.remove_root_at(myobj_idx);

          mem.at_put(slot, 3, fill);
        }
      }
    }
    count += 1;
    if count % 10 == 0 {
      print!("{}.", count);
      let r = stdout().flush();
      if r.is_err() {
        print!("Error {} occured\n", r.unwrap_err());
      }
    }
    if count % 100  == 0 {
      print!("\n");
    }
  }

  print!("\n");
  mem.print_gc_stats();

  mem.gc();
  let liveset:usize = mem.live_objects().map(|o| mem.element_size(o) + gc::OBJECT_HEADER_SLOTS).sum();
  println!( "Post GC - Current Heap has  {} objects with {} slots", mem.into_iter().count(), liveset);

  mem.remove_root_at(root_idx);
  mem.gc();
  let freed = mem.compact();
  println!("Compact freed {} segments", freed);
  let liveset:usize = mem.live_objects().map(|o| mem.element_size(o) + gc::OBJECT_HEADER_SLOTS).sum();
  println!( "No Roots - Current Heap has  {} objects with {} slots", mem.into_iter().count(), liveset);

  println!( "Run Heap Size is {}", mem.total_heap_slots());
  mem.print_freelist();

}

#[cfg(test)]
mod tests {
    use super::gc;
    use super::rnd;

    // -----------------------------------------------------------------------
    // Test 1: existing stress test — random object graph replacement,
    // verify pointer invariants survive GC and compaction.
    // -----------------------------------------------------------------------
    #[test]
    fn test_stress_gc() {
        const LOOP: usize = 1000;
        const MAX_OBJECT_SIZE: usize = 32;
        const NUMBER_OF_ROOTS: usize = 128;

        let mut mem = gc::Memory::initialze_memory();
        let root = mem.allocate_object(NUMBER_OF_ROOTS);
        let root_idx = mem.add_root(root);

        for i in 0..mem.element_size(mem.get_root(root_idx)) {
            let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
            let root = mem.get_root(root_idx);
            mem.at_put(root, i, allocated);
        }

        let mut count = 0;
        while count < LOOP {
            let root = mem.get_root(root_idx);
            for i in 0..mem.element_size(root) {
                let root = mem.get_root(root_idx);
                if i == rnd::rnd_sz(mem.element_size(root)) {
                    let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
                    let root = mem.get_root(root_idx);
                    mem.at_put(root, i, allocated);
                } else {
                    let root = mem.get_root(root_idx);
                    let mut myobj = mem.at(root, i);
                    for j in 0..mem.element_size(myobj) {
                        let prev = mem.at(myobj, j);
                        if prev != 0 {
                            assert_eq!(mem.at(prev, 0), prev, "slot should point to self");
                            assert_eq!(mem.at(prev, 1), myobj, "slot should point to outer obj");
                            assert!(mem.element_size(mem.at(prev, 2)) >= 2, "fill2 too small");
                            assert!(mem.element_size(mem.at(prev, 3)) >= 3, "fill3 too small");
                        }
                        let myobj_idx = mem.add_root(myobj);
                        let slot = mem.allocate_object(4);
                        myobj = mem.get_root(myobj_idx);
                        mem.remove_root_at(myobj_idx);

                        mem.at_put(myobj, j, slot);
                        mem.at_put(slot, 0, slot);
                        mem.at_put(slot, 1, myobj);

                        let myobj_idx = mem.add_root(myobj);
                        let slot_idx  = mem.add_root(slot);
                        let fill = mem.allocate_object(2);
                        let mut slot = mem.get_root(slot_idx);
                        myobj = mem.get_root(myobj_idx);
                        mem.remove_root_at(slot_idx);
                        mem.remove_root_at(myobj_idx);
                        mem.at_put(slot, 2, fill);

                        let myobj_idx = mem.add_root(myobj);
                        let slot_idx  = mem.add_root(slot);
                        let fill = mem.allocate_object(3);
                        slot = mem.get_root(slot_idx);
                        myobj = mem.get_root(myobj_idx);
                        mem.remove_root_at(slot_idx);
                        mem.remove_root_at(myobj_idx);
                        mem.at_put(slot, 3, fill);
                    }
                }
            }
            count += 1;
        }

        mem.gc();
        let live_before = mem.live_objects().count();
        assert!(live_before > 0, "should have live objects after GC with roots");

        mem.remove_root_at(root_idx);
        mem.gc();
        let live_after = mem.live_objects().count();
        assert_eq!(live_after, 0, "all objects should be dead after removing root");

        let freed = mem.compact();
        assert!(freed > 0, "compact should free segments after all objects dead");
        assert_eq!(mem.segment_count(), 1, "should collapse to a single segment");
    }

    // -----------------------------------------------------------------------
    // Test 2: build a large live set spanning many segments via an array
    // root, then reduce it so most objects become garbage, then verify that
    // GC + compact shrinks the segment count back toward 1.
    // -----------------------------------------------------------------------
    #[test]
    fn test_segment_reduction() {
        // Size the array so filling it forces many segments to be opened.
        // Each entry is a 2-slot object (4 slots total with header).
        // One segment holds ~255 such objects; 200 array slots × 4-object
        // chains = 800 objects ≈ 3–4 segments per array fill.
        const ARRAY_SIZE: usize = 600;
        const CHAIN: usize = 4; // objects per array slot

        let mut mem = gc::Memory::initialze_memory();

        // Allocate the root array and register it.
        let arr = mem.allocate_object(ARRAY_SIZE);
        let arr_idx = mem.add_root(arr);

        // Fill every slot with a small chain of objects so we spread across
        // many segments.
        for i in 0..ARRAY_SIZE {
            let arr = mem.get_root(arr_idx);
            let arr_idx2 = mem.add_root(arr);
            let head = mem.allocate_object(CHAIN);
            let arr = mem.get_root(arr_idx2);
            mem.remove_root_at(arr_idx2);
            mem.at_put(arr, i, head);
            // hang a few more objects off head so each entry spans a few objects
            let head_idx = mem.add_root(head);
            for k in 0..CHAIN {
                let child = mem.allocate_object(32);
                let head = mem.get_root(head_idx);
                mem.at_put(head, k, child);
            }
            mem.remove_root_at(head_idx);
        }

        let segs_at_peak = mem.segment_count();
        println!("segs at peak: {}", segs_at_peak);
        assert!(segs_at_peak > 1, "filling {} slots should span multiple segments", ARRAY_SIZE);
        println!("test_segment_reduction: peak {} segments", segs_at_peak);

        // GC with full live set — nothing should be collected.
        mem.gc();
        let live_at_peak = mem.live_objects().count();
        println! ("test_segment_reduction: live at peak {}", live_at_peak);
        assert!(live_at_peak > 0, "live set should survive GC");

        // Null out 90% of the array slots → those subtrees become garbage.
        let keep = ARRAY_SIZE / 10;
        let arr = mem.get_root(arr_idx);
        for i in keep..ARRAY_SIZE {
            mem.at_put(arr, i, 0);
        }

        mem.gc();
        let live_reduced = mem.live_objects().count();
        assert!(live_reduced < live_at_peak, "live set should shrink after nulling 90% of slots");
        println!("test_segment_reduction: live before={} after nulling 90%={}",
            live_at_peak, live_reduced);

        let segs_before_compact = mem.segment_count();
        let freed = mem.compact();
        let segs_after_compact = mem.segment_count();

        println!("test_segment_reduction: {} segs before compact, freed {}, {} remain",
            segs_before_compact, freed, segs_after_compact);

        assert!(freed > 0, "compact should free segments after reducing live set");
        assert!(segs_after_compact < segs_before_compact, "segment count should decrease after compact");

        // Remove root and verify everything is collected.
        mem.remove_root_at(arr_idx);
        mem.gc();
        let freed2 = mem.compact();
        assert_eq!(mem.live_objects().count(), 0, "no live objects after removing root");
        assert!(freed2 > 0 || mem.segment_count() == 1, "should collapse to 1 segment");
        println!("test_segment_reduction: final {} segments", mem.segment_count());
    }
}
