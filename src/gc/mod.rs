use std::time::SystemTime;

const MAX_MEMORY_SLOTS: usize = 1024 * 128;
pub const OBJECT_HEADER_SLOTS: usize = 1;

pub struct Memory {
  head: usize,
  mem: [usize; MAX_MEMORY_SLOTS],
  roots: Vec<usize>,
  gc_count: usize,
  allocates: usize,
  last_gc_ms: u128,
  total_gc_ms: u128,
  lastgc_live_mem: usize,
  lastgc_free_mem: usize,
  sum_mem_collected: usize,
  show_gc: bool,
  show_allocates: bool,
  show_heap_map: bool,
  show_free_list: bool,
}

impl<'a> IntoIterator for &'a Memory {
  type Item = usize;
  type IntoIter = MemoryIntoIterator<'a>;
  fn into_iter(self) -> Self::IntoIter {
    MemoryIntoIterator {
      mem: self,
      scan: 0,
      free: 0,
    }
  }
}

pub struct MemoryIntoIterator<'a> {
  mem: &'a Memory,
  scan: usize,
  free: usize,
}

impl<'a> Iterator for MemoryIntoIterator<'a> {
  type Item = usize;
  fn next(&mut self) -> Option<Self::Item> {
    if self.scan == 0 {
      self.scan = 1;
      self.free = self.mem.head;
    } else {
      self.scan = self.mem.next_object_in_heap(self.scan);
    }
    while self.scan == self.free {
      self.scan = self.mem.next_object_in_heap(self.free);
      self.free = self.mem.get_fl_next(self.free);
    }
    if self.scan >= MAX_MEMORY_SLOTS - 1 {
      return None;
    } else {
      return Some(self.scan);
    }
  }
}

impl Memory {
  pub fn initialze_memory() -> Memory {
    let mut mem = Memory {
      head: 1,
      mem: [0; MAX_MEMORY_SLOTS],
      roots: Vec::new(),
      gc_count: 0,
      allocates: 0,
      lastgc_live_mem: 0,
      lastgc_free_mem: 0,
      last_gc_ms: 0,
      total_gc_ms: 0,
      sum_mem_collected: 0,
      show_gc: false,
      show_allocates: false,
      show_heap_map: false,
      show_free_list: false,
    };
    mem.set_size(0, MAX_MEMORY_SLOTS); // magic memory at zero is heap_size
    mem.set_size(mem.head, MAX_MEMORY_SLOTS - 2); // set initial object size as all heap
    mem.set_fl_next(mem.head, 0);
    (mem)
  }

  // objects API
  // allocate_object (size) --- size is number of indexable slots
  // add/remote_root () --- add to or remove from gc root set.
  // element_size() - number of indexable slots - get_size() - OBJECT_HEADER_SLOTS
  // at_put - store into object slot at index
  // at -- fetch object slot at index

  pub fn allocate_object(&mut self, unrounded_size: usize) -> usize {
    self.allocates += 1;
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
  pub fn enable_show_heap_map(&mut self, enabled: bool) {
    self.show_heap_map = enabled;
  }
  pub fn enable_show_freelist(&mut self, enabled: bool) {
    self.show_free_list = enabled;
  }
  pub fn enable_show_gc(&mut self, enabled: bool) {
    self.show_gc = enabled;
  }
  pub fn enable_show_allocates(&mut self, enabled: bool) {
    self.show_allocates = enabled;
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
    let mut free = self.head;
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
        if self.show_allocates {
          println!(
            "Success: allocate_object returning -> {} size {}",
            new_object, size
          );
        }
        if self.head != free {
          if self.show_allocates {
            println!("Reset head past intermediate free blocks \n");
            let mut show = self.head;
            while show != free {
              println!("Abandon {} size {}\n", show, self.get_size(show));
              show = self.get_fl_next(show);
            }
          }
          self.head = free;
        }
        return new_object;
      }
      free = self.get_fl_next(free);
    }
    (0)
  }

  pub fn gc(&mut self) {
    let start = SystemTime::now();
    for i in 0..self.roots.len() {
      self.mark_and_scan(self.roots[i]);
    }
    self.sweep();
    self.gc_count += 1;
    if self.show_gc {
      self.print_gc_stats();
    }
    match start.elapsed() {
      Ok(elapsed) => {
        self.last_gc_ms = elapsed.as_millis();
        self.total_gc_ms += self.last_gc_ms;
      }
      Err(e) => {
        println!("Error: {:?}", e);
      }
    }
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
          self.head = scan;
          self.set_fl_next(scan, 0);
          tail = scan;
        } else {
          if self.next_object_in_heap(tail) == scan {
            self.set_size(tail, self.get_size(tail) + self.get_size(scan));
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
    if self.show_free_list {
      self.print_freelist();
    }
    if self.show_heap_map {
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

  pub fn print_gc_stats(&self) {
    println!(
    "{} gcs, {} object allocates, Last GC: Live {} Dead {} in {} ms, Lifetime Collected {} in {} ms\n",
    self.gc_count,
    self.allocates,
    self.lastgc_live_mem,
    self.lastgc_free_mem,
    self.last_gc_ms,
    self.sum_mem_collected,
    self.total_gc_ms,
  );
  }

  fn print_heap(&mut self) {
    print!("\x1B[{};{}H", 1, 1);
    let mut free = self.head;
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
    self.print_gc_stats();
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
