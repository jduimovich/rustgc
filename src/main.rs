mod gc;
mod rnd;

use std::env;

const LOOP: usize = 50000;
const MAX_OBJECT_SIZE: usize = 32;
const NUMBER_OF_ROOTS: usize = 32;

fn main() {
  let args: Vec<String> = env::args().collect(); 

  let mut mem = gc::Memory::initialze_memory();

  for i in 0..args.len() { 
    if args[i] == "debug" { mem.enable_debug(true);}
    if args[i] == "heap" { mem.enable_print_heap(true);}
  }
  let mut objects = Vec::new();

  // poof up some roots
  for i in 0..NUMBER_OF_ROOTS {
    let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
    objects.push(allocated);
    mem.add_root(allocated);
  }

  let mut count = 0;
  while count < LOOP {
    for i in 0..objects.len() {
      let myobj = objects[i];
      if i == rnd::rnd_sz(objects.len()) {
        let allocated = mem.allocate_object(rnd::rnd_sz(MAX_OBJECT_SIZE));
        mem.remove_root(objects[i]);
        mem.add_root(allocated);
        objects[i] = allocated;
      } else {
        for j in 0..mem.element_size(myobj) {
          let prev = mem.at(myobj, j);
          if prev != 0 {
            // println!(
            //   "MyObj {} Prev {} size {}  at:0 {} at:1 {}\n", myobj, prev, mem.element_size(prev) ,
            //   mem.at(prev, 0), mem.at(prev, 1));
            if mem.at(prev, 0) !=  prev { panic!("object should point to self");}
            if mem.at(prev, 1) !=  myobj { panic!("object should point to outerobj");}
          }
          let slot = mem.allocate_object(2); // fill with small objects
          mem.at_put(myobj, j, slot);
          mem.at_put(slot, 0, slot);
          mem.at_put(slot, 1, myobj);
        }
      }
    }
    count += 1;
  }
  for i in 0..NUMBER_OF_ROOTS { 
    mem.remove_root(objects[i]);
    mem.gc ();
  }
  mem.print_freelist();

}
