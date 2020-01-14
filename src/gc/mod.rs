const MAX_MEMORY_SLOTS: usize = 1024 * 4;
const OBJECT_HEADER_SLOTS: usize = 1;

pub struct Memory {
  head: usize,
  mem: [usize; MAX_MEMORY_SLOTS],
  roots: Vec<usize>,
  gc_count: usize,
  lastgc_live_mem: usize,
  lastgc_free_mem: usize,
  sum_mem_collected: usize,
  debug: bool,
  showHeapMap: bool,
}

impl Memory {
  pub fn initialze_memory() -> Memory {
    let mut mem = Memory {
      head: 1,
      mem: [0; MAX_MEMORY_SLOTS],
      roots: Vec::new(),
      gc_count: 0,
      lastgc_live_mem: 0,
      lastgc_free_mem: 0,
      sum_mem_collected: 0,
      debug: false,
      showHeapMap: false,
    };
    mem.set_size(0, MAX_MEMORY_SLOTS); // magic memory at zero is heap_size
    mem.set_size(mem.head, MAX_MEMORY_SLOTS - 2); // set initial object size as all heap
    mem.set_fl_next(mem.head, 0);
    (mem)
  }

  // objects are allocated
  // element_size - number of slots, indexed - 0..element_size
  // at_put - story into object slot
  // at -- fetch slot

  pub fn allocate_object(&mut self, unrounded_size: usize) -> usize {
    let mut result = self.allocate_object_nocompress(unrounded_size);
    if result == 0 {
      self.gc();
      result = self.allocate_object_nocompress(unrounded_size);
      if result == 0 {
        self.print_freelist();
        self.print_heap();
        panic!("out of memory");
      }
    }
    (result)
  }
  pub fn add_root(&mut self, obj: usize) {
    self.roots.push(obj);
  }
  pub fn remove_root(&mut self, obj: usize) {
    for i in 0..self.roots.len() {
      if obj == self.roots[i] {
        self.roots.remove(i);
        return;
      }
    }
  }

  pub fn at_put(&mut self, obj: usize, index: usize, value: usize) {
    let slot = OBJECT_HEADER_SLOTS + index;
    if slot >= self.mem[obj] {
      panic!("index out of range");
    }
    self.mem[obj + slot] = value;
  }

  pub fn at(&self, obj: usize, index: usize) -> usize {
    let slot = OBJECT_HEADER_SLOTS + index;
    if slot >= self.mem[obj] {
      panic!("index out of range");
    }
    return self.mem[obj + slot];
  }
  pub fn element_size(&self, obj: usize) -> usize {
    return self.mem[obj] - OBJECT_HEADER_SLOTS;
  }

  pub fn enable_debug(&mut self, enabled: bool) {
    self.debug = enabled;
  }

  pub fn enable_print_heap(&mut self, enabled: bool) {
    self.showHeapMap = enabled;
  }

  fn rounded_size(unrounded_size: usize) -> usize {
    (unrounded_size + 1) & !(1) // rounded to 2
  }

  fn get_size(&self, obj: usize) -> usize {
    return self.mem[obj];
  }
  fn set_size(&mut self, obj: usize, size: usize) {
    self.mem[obj] = size;
  }
  fn next_object_in_heap(&self, obj: usize) -> usize {
    return obj + self.get_size(obj);
  }
  fn merge_two_objects(&mut self, first: usize, second: usize) {
    self.set_size(first, self.get_size(first) + self.get_size(second));
  }
  fn get_free(&self) -> usize {
    return self.head;
  }
  //free list is linked off the first slot
  fn get_fl_next(&self, obj: usize) -> usize {
    return self.mem[obj + 1];
  }
  fn set_fl_next(&mut self, obj: usize, next: usize) {
    self.mem[obj + 1] = next;
  }
  fn mark_object(&mut self, obj: usize) {
    self.mem[obj] += 1;
  }
  fn unmark_object(&mut self, obj: usize) {
    self.mem[obj] -= 1;
  }
  fn is_marked(&self, obj: usize) -> bool {
    (self.mem[obj] & 1) != 0
  }

  fn allocate_object_nocompress(&mut self, unrounded_size: usize) -> usize {
    let size = Memory::rounded_size(unrounded_size + OBJECT_HEADER_SLOTS);
    let mut free = self.get_free();
    while free != 0 {
      let avail = self.get_size(free);
      if avail > size {
        let newsize = avail - size;
        if newsize < 2 {
          panic!("remaining size is less than 2");
        }
        // shrink current free to smaller size
        self.set_size(free, newsize);
        // new object is on the end of current free object
        let new_object = free + newsize;
        self.set_size(new_object, size);
        for index in 0..self.element_size(new_object) {
          self.at_put(new_object, index, 0);
        }
        if self.debug {
          println!(
            "Success: allocate_object returning -> {} size {}",
            new_object, size
          );
        }
        return new_object;
      }
      free = self.get_fl_next(free);
    }
    (0)
  }

  pub fn gc(&mut self) {
    for i in 0..self.roots.len() {
      self.mark_and_scan(self.roots[i]);
    }
    self.sweep();
    self.gc_count += 1;
    
      println!(
        "Count {} Live  {} FreeMem {} Collected {} \n\n",
        self.gc_count, self.lastgc_live_mem, self.lastgc_free_mem, self.sum_mem_collected
      );
    
  }
  fn sweep(&mut self) {
    let mut scan = 1;
    self.head = 0;
    let mut tail = self.head;

    self.lastgc_free_mem = 0;
    self.lastgc_live_mem = 0;
    while scan < MAX_MEMORY_SLOTS - 1 {
      if self.is_marked(scan) {
        self.unmark_object(scan);
        self.lastgc_live_mem += self.get_size(scan);
      } else {
        self.lastgc_free_mem += self.get_size(scan);
        if tail == 0 {
          // first free object in memory order
          self.head = scan;
          self.set_fl_next(scan, 0);
          tail = scan;
        } else {
          if self.next_object_in_heap(tail) == scan {
            self.merge_two_objects(tail, scan);
          } else {
            self.set_fl_next(tail, scan);
            self.set_fl_next(scan, 0);
            tail = scan;
          }
        }
      }
      scan = self.next_object_in_heap(scan);
    }
    self.sum_mem_collected += self.lastgc_free_mem;
    if self.debug {
      self.print_freelist();
    }
    if self.showHeapMap {
      self.print_heap();
    }
  }

  fn mark_and_scan(&mut self, object: usize) {
    if object == 0 || self.is_marked(object) {
      return;
    }
    let slots = self.get_size(object);
    self.mark_object(object);
    for i in OBJECT_HEADER_SLOTS..slots {
      self.mark_and_scan(self.mem[object + i]);
    }
  }

  fn print_heap(&mut self) {
    print!("\x1B[{};{}H", 1, 1);
    let mut free = self.get_free();
    while free != 0 {
      self.mark_object(free);
      free = self.get_fl_next(free);
    }
    let mut scan = 1;
    let mut count = 0;
    while scan < MAX_MEMORY_SLOTS - 1 {
      let c;
      if self.is_marked(scan) {
        self.unmark_object(scan);
        c = 'x';
      } else {
        c = '.';
      }
      for _i in 1..self.get_size(scan) {
        print!("{}", c);
        count += 1;
        if count % 120 == 0 {
          print!("\n");
        }
      }
      scan = self.next_object_in_heap(scan);
    }
    println!("\n");
    println!(
      "print_heap: Count {} Live  {} FreeMem {}  Collected {} \n\n",
      self.gc_count, self.lastgc_live_mem, self.lastgc_free_mem, self.sum_mem_collected
    );
  }

  pub fn print_freelist(&mut self) {
    println!("\nprint_freelist: Head = {}", self.head);
    let mut free = self.head;
    let mut count = 0;
    let mut total_free = 0;
    while free != 0 {
      let size = self.get_size(free);
      let next = self.get_fl_next(free);
      total_free += self.get_size(free);
      println!("{}: Free = {} {} slots  next = {}", count, free, size, next);
      free = next;
      count += 1;
      if count > MAX_MEMORY_SLOTS {
        panic!()
      }
    }
    println!(
      "print_freelist {} elements, total free = {}\n",
      count, total_free
    );
  }
}
