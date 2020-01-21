mod gc;
mod rnd;
use std::io::stdout;
use std::io::Write;

use std::env;

const LOOP: usize = 20000;
const MAX_OBJECT_SIZE: usize = 128;
const NUMBER_OF_ROOTS: usize = 32;

fn main() {

  let args: Vec<String> = env::args().collect();

  let mut mem = gc::Memory::initialze_memory();
  println!( "New Heap  {} objects", mem.live_objects().count()); 
  for obj in mem.live_objects()  { 
    println!( "New Heap, iterate over objects: {}", obj); 
  }

  for i in 0..args.len() {
    if args[i] == "allocates" {
      mem.enable_show_allocates(true);
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
    if args[i] == "help" {
      println!(
        "GC Demo Options: 
        \t allocates (default off) - show all allocates\n
        \t freelist (default off) - show freelist every gc \n
        \t heap (default off) - show heap every gc\n
        \t gc (default off) - summary  every gc\n
        "
      );
      return;
    }
  }
  let root = mem.allocate_object(NUMBER_OF_ROOTS);
  mem.add_root(root);
  // fill in more objects off this single root
  for i in 0..mem.element_size(root) { 
    let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
    mem.at_put(root, i, allocated);  
  }

  let mut count = 0;
  while count < LOOP {
    for i in 0..mem.element_size(root) {
      let myobj = mem.at(root,i);
      if i == rnd::rnd_sz(mem.element_size(root)) {
        let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));  
        mem.at_put(root, i, allocated);  
      } else {
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
          let slot = mem.allocate_object(4); // fill with small objects
          mem.at_put(myobj, j, slot);
          mem.at_put(slot, 0, slot);
          mem.at_put(slot, 1, myobj);
          let fill = mem.allocate_object(2);
          mem.at_put(slot, 2, fill);
          let fill = mem.allocate_object(3);
          mem.at_put(slot, 3, fill);
        }
      }
    }
    count += 1;
    if count % 100 == 0 {
      print!("{}.", count);
      let r = stdout().flush();
      if r.is_err() {
        print!("Error {} occured\n", r.unwrap_err());
      }
    }
    if count % 1000 == 0 {
      print!("\n"); 
      mem.print_gc_stats();
      let mut size = 0;
      for obj in &mem  { 
        size += mem.element_size(obj) + gc::OBJECT_HEADER_SLOTS; 
      }
      println!( "Current Heap has  {} objects with {} slots", mem.into_iter().count(), size); 
    }
  } 

  mem.gc();   
  let mut size = 0;
  for obj in &mem  { 
    size += mem.element_size(obj) + gc::OBJECT_HEADER_SLOTS; 
  }
  println!( "Post GC - Current Heap has  {} objects with {} slots", mem.into_iter().count(), size);  
  
  mem.remove_root(root); 
  mem.gc();  
  let mut size = 0; 
  for obj in mem.live_objects()  { 
    size += mem.element_size(obj) + gc::OBJECT_HEADER_SLOTS;  
  } 
  println!( "No Roots - Post GC Current Heap has  {} objects with {} slots", mem.into_iter().count(), size);  
  mem.print_freelist();
}
