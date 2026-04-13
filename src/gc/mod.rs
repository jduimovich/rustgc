use std::time::SystemTime;

// --- pointer / segment layout ---
const SEGMENT_BITS: usize = 8;
const SEGMENT_SHIFT: usize = 64 - SEGMENT_BITS; // 56
const SLOT_MASK: usize = (1usize << SEGMENT_SHIFT) - 1;

// --- heap dimensions (tune at compile time) ---
const SEGMENT_SLOTS: usize = 65536; // 256KB per segment (assuming 8-byte slots)
pub const MAX_SEGMENTS: usize = 32;

// --- mark bits per segment ---
type Bits = u128;
const MARK_BITS_PER_WORD: usize = 128;
const MARK_WORDS_PER_SEGMENT: usize =
    (SEGMENT_SLOTS + MARK_BITS_PER_WORD - 1) / MARK_BITS_PER_WORD;

pub const OBJECT_HEADER_SLOTS: usize = 1;

pub type Obj = usize;

// ---------------------------------------------------------------------------
// Pointer encoding helpers
// ---------------------------------------------------------------------------

#[inline(always)]
pub fn encode_ptr(seg: usize, offset: usize) -> Obj {
    (seg << SEGMENT_SHIFT) | offset
}

#[inline(always)]
fn ptr_segment(obj: Obj) -> usize {
    obj >> SEGMENT_SHIFT
}

#[inline(always)]
fn ptr_offset(obj: Obj) -> usize {
    obj & SLOT_MASK
}

#[inline(always)]
fn is_null(obj: Obj) -> bool {
    obj == 0
}

// Header encodes owning segment in high bits and slot count in low bits.
#[inline(always)]
fn encode_header(seg: usize, size_in_slots: usize) -> usize {
    encode_ptr(seg, size_in_slots)
}

#[inline(always)]
fn header_size(raw: usize) -> usize {
    ptr_offset(raw)
}

// ---------------------------------------------------------------------------
// Segment
// ---------------------------------------------------------------------------

struct Segment {
    mem: Box<[usize; SEGMENT_SLOTS]>,
    /// Per-segment free-list head (fully encoded Obj), 0 = empty.
    free_head: Obj,
    /// Next offset available for bump allocation.
    bump: usize,
    bump_exhausted: bool,
    mark_bits: [Bits; MARK_WORDS_PER_SEGMENT],
}

impl Segment {
    fn new() -> Self {
        Segment {
            mem: Box::new([0usize; SEGMENT_SLOTS]),
            free_head: 0,
            bump: 1, // offset 0 reserved as null sentinel
            bump_exhausted: false,
            mark_bits: [0; MARK_WORDS_PER_SEGMENT],
        }
    }

    #[inline(always)]
    fn read(&self, offset: usize) -> usize {
        self.mem[offset]
    }

    #[inline(always)]
    fn write(&mut self, offset: usize, val: usize) {
        self.mem[offset] = val;
    }

    fn mark(&mut self, offset: usize) {
        self.mark_bits[offset / MARK_BITS_PER_WORD] |= 1 << (offset % MARK_BITS_PER_WORD);
    }

    fn is_marked(&self, offset: usize) -> bool {
        (self.mark_bits[offset / MARK_BITS_PER_WORD] & (1 << (offset % MARK_BITS_PER_WORD))) != 0
    }

    fn clear_marks(&mut self) {
        self.mark_bits.fill(0);
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

pub struct Memory {
    segments: Vec<Segment>,
    seg_count: usize,
    current_seg: usize,

    roots: Vec<Obj>,
    mark_stack: Vec<Obj>,

    gc_count: usize,
    allocates: usize,
    last_gc_ms: u128,
    total_gc_ms: u128,
    lastgc_live_mem: usize,
    lastgc_free_mem: usize,
    show_gc: bool,
    show_allocates: bool,
    show_abandons: bool,
    show_heap_map: bool,
    show_free_list: bool,
    show_compact: bool,
}

// ---------------------------------------------------------------------------
// Iterator over live objects
// ---------------------------------------------------------------------------

pub struct MemoryIntoIterator<'a> {
    mem: &'a Memory,
    seg_idx: usize,
    scan_off: usize,
    /// Offsets of free-block starts in the current segment (sorted).
    free_offsets: Vec<usize>,
}

impl<'a> MemoryIntoIterator<'a> {
    fn load_free_offsets(&mut self) {
        self.free_offsets.clear();
        if self.seg_idx >= self.mem.seg_count {
            return;
        }
        let mut free = self.mem.segments[self.seg_idx].free_head;
        while !is_null(free) {
            self.free_offsets.push(ptr_offset(free));
            free = self.mem.get_fl_next(free);
        }
        self.free_offsets.sort_unstable();
    }
}

impl<'a> IntoIterator for &'a Memory {
    type Item = Obj;
    type IntoIter = MemoryIntoIterator<'a>;
    fn into_iter(self) -> Self::IntoIter {
        let mut it = MemoryIntoIterator {
            mem: self,
            seg_idx: 0,
            scan_off: 1,
            free_offsets: Vec::new(),
        };
        it.load_free_offsets();
        it
    }
}

impl<'a> Iterator for MemoryIntoIterator<'a> {
    type Item = Obj;
    fn next(&mut self) -> Option<Obj> {
        loop {
            if self.seg_idx >= self.mem.seg_count {
                return None;
            }
            let seg = &self.mem.segments[self.seg_idx];
            let upper = if seg.bump_exhausted {
                SEGMENT_SLOTS
            } else {
                seg.bump
            };
            if self.scan_off >= upper {
                self.seg_idx += 1;
                self.scan_off = 1;
                self.load_free_offsets();
                continue;
            }
            let off = self.scan_off;
            let obj = encode_ptr(self.seg_idx, off);
            let size = self.mem.get_size(obj);
            if size == 0 {
                return None;
            }
            self.scan_off += size;
            if self.free_offsets.binary_search(&off).is_ok() {
                continue; // skip free block
            }
            return Some(obj);
        }
    }
}

// ---------------------------------------------------------------------------
// Memory impl
// ---------------------------------------------------------------------------

impl Memory {
    pub fn initialze_memory() -> Memory {
        let mut m = Memory {
            segments: Vec::with_capacity(MAX_SEGMENTS),
            seg_count: 0,
            current_seg: 0,
            roots: Vec::new(),
            mark_stack: Vec::new(),
            gc_count: 0,
            allocates: 0,
            last_gc_ms: 0,
            total_gc_ms: 0,
            lastgc_live_mem: 0,
            lastgc_free_mem: 0,
            show_gc: false,
            show_allocates: false,
            show_abandons: false,
            show_heap_map: false,
            show_free_list: false,
            show_compact: false,
        };
        m.add_segment();
        m
    }

    fn add_segment(&mut self) -> usize {
        assert!(self.seg_count < MAX_SEGMENTS, "segment table full");
        let idx = self.seg_count;
        self.segments.push(Segment::new());
        self.seg_count += 1;
        idx
    }

    pub fn total_heap_slots(&self) -> usize {
        self.seg_count * SEGMENT_SLOTS
    }

    pub fn segment_count(&self) -> usize {
        self.seg_count
    }

    /// Free slots in a segment = free-list slots + unallocated bump space.
    fn segment_free_slots(&self, seg_idx: usize) -> usize {
        let seg = &self.segments[seg_idx];
        let bump_free = if seg.bump_exhausted { 0 } else { SEGMENT_SLOTS - seg.bump };
        let mut free_list_slots = 0usize;
        let mut f = seg.free_head;
        while !is_null(f) {
            free_list_slots += header_size(seg.read(ptr_offset(f)));
            f = seg.read(ptr_offset(f) + 1);
        }
        bump_free + free_list_slots
    }

    // --- public object API ---

    pub fn allocate_object(&mut self, unrounded_size: usize) -> Obj {
        self.allocates += 1;
        let size = Self::rounded_size(unrounded_size + OBJECT_HEADER_SLOTS);

        // 1. bump alloc in current segment
        if let Some(obj) = self.bump_alloc(size) {
            if self.show_allocates {
                println!("alloc(bump) -> {:x} size {}", obj, size);
            }
            return obj;
        }

        // 2. open a fresh segment
        if self.seg_count < MAX_SEGMENTS {
            let new_seg = self.add_segment();
            self.current_seg = new_seg;
            if let Some(obj) = self.bump_alloc(size) {
                if self.show_allocates {
                    println!("alloc(bump/new seg) -> {:x} size {}", obj, size);
                }
                return obj;
            }
        }

        // 3. GC
        self.gc();
        self.compact();

        // Compact only if the current segment is >= 80% full (free < 20%).
        // This keeps segment count down without compacting on every collection.
        // let free_in_cur = self.segment_free_slots(self.current_seg);
        // if free_in_cur * 100 < SEGMENT_SLOTS * 20 {
        //     self.compact();
        // }

        // 4. free-list alloc across all segments
        if let Some(obj) = self.freelist_alloc(size) {
            if self.show_allocates {
                println!("alloc(freelist) -> {:x} size {}", obj, size);
            }
            return obj;
        }

        self.print_freelist();
        self.print_heap();
        panic!("out of memory");
    }

    pub fn live_objects(&self) -> MemoryIntoIterator<'_> {
        self.into_iter()
    }

    pub fn add_root(&mut self, obj: Obj) -> usize {
        let idx = self.roots.len();
        self.roots.push(obj);
        idx
    }

    pub fn get_root(&self, idx: usize) -> Obj {
        self.roots[idx]
    }

    /// Remove the root at a specific index.  Use in reverse (LIFO) order
    /// relative to add_root so that swap_remove only displaces the last slot.
    pub fn remove_root_at(&mut self, idx: usize) {
        self.roots.swap_remove(idx);
    }

    pub fn remove_root(&mut self, obj: Obj) {
        if let Some(i) = self.roots.iter().position(|&r| r == obj) {
            self.roots.swap_remove(i);
        }
    }

    pub fn at_put(&mut self, obj: Obj, index: usize, value: Obj) {
        let (s, o) = Self::decode(obj);
        let slots: Obj = header_size(self.segments[s].read(o));
        let base = o + OBJECT_HEADER_SLOTS;
        assert!(index < slots, "at_put: index {} out of bounds (slots {})", index, slots);
        self.segments[s].write(base + index, value);
    }

    pub fn at(&self, obj: Obj, index: usize) -> Obj {
        let (s, o) = Self::decode(obj);
        let slots: Obj = header_size(self.segments[s].read(o));
        let base = o + OBJECT_HEADER_SLOTS;
        assert!(index < slots, "at: index {} out of bounds (slots {})", index, slots);
        self.segments[s].read(base + index)
    }

    pub fn element_size(&self, obj: Obj) -> usize {
        self.get_size(obj) - OBJECT_HEADER_SLOTS
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
    pub fn enable_show_abandons(&mut self, enabled: bool) {
        self.show_abandons = enabled;
    }
    pub fn enable_show_compact(&mut self, enabled: bool) {
        self.show_compact = enabled;
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
        if let Ok(elapsed) = start.elapsed() {
            self.last_gc_ms = elapsed.as_millis();
            self.total_gc_ms += self.last_gc_ms;
        }
    }

    pub fn print_gc_stats(&self) {
        println!(
            "{} gcs, {} allocates, segs {}/{}, Last GC: live {} dead {} in {} ms, total GC {} ms",
            self.gc_count,
            self.allocates,
            self.seg_count,
            MAX_SEGMENTS,
            self.lastgc_live_mem,
            self.lastgc_free_mem,
            self.last_gc_ms,
            self.total_gc_ms,
        );
    }

    pub fn print_freelist(&self) {
        println!("\nprint_freelist ({} segments):", self.seg_count);
        let mut total_free = 0usize;
        let mut count = 0usize;
        for s in 0..self.seg_count {
            let mut free = self.segments[s].free_head;
            while !is_null(free) {
                let size = self.get_size(free);
                let next = self.get_fl_next(free);
                total_free += size;
                println!(
                    "  {}: seg {} off {} size {}  next {:x}",
                    count,
                    s,
                    ptr_offset(free),
                    size,
                    next
                );
                free = next;
                count += 1;
            }
        }
        println!("print_freelist {} blocks, total free = {}\n", count, total_free);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    #[inline(always)]
    fn decode(obj: Obj) -> (usize, usize) {
        (ptr_segment(obj), ptr_offset(obj))
    }

    fn rounded_size(unrounded_size: usize) -> usize {
        (unrounded_size + 1) & !1
    }

    fn get_size(&self, obj: Obj) -> usize {
        let (s, o) = Self::decode(obj);
        header_size(self.segments[s].read(o))
    }

    fn set_size(&mut self, obj: Obj, size: usize) {
        let (s, o) = Self::decode(obj);
        let encoded = encode_header(s, size);
        self.segments[s].write(o, encoded);
    }

    fn get_fl_next(&self, obj: Obj) -> Obj {
        let (s, o) = Self::decode(obj);
        self.segments[s].read(o + 1)
    }

    fn set_fl_next(&mut self, obj: Obj, next: Obj) {
        let (s, o) = Self::decode(obj);
        self.segments[s].write(o + 1, next);
    }

    fn next_object_in_segment(&self, obj: Obj) -> Obj {
        let (s, o) = Self::decode(obj);
        encode_ptr(s, o + self.get_size(obj))
    }

    // -----------------------------------------------------------------------
    // Allocation
    // -----------------------------------------------------------------------

    fn bump_alloc(&mut self, size: usize) -> Option<Obj> {
        let seg_idx = self.current_seg;
        let seg = &mut self.segments[seg_idx];
        if seg.bump_exhausted {
            return None;
        }
        let start = seg.bump;
        if start + size > SEGMENT_SLOTS {
            seg.bump_exhausted = true;
            return None;
        }
        seg.bump += size;
        let encoded_hdr = encode_header(seg_idx, size);
        seg.mem[start] = encoded_hdr;
        seg.mem[start + OBJECT_HEADER_SLOTS..start + size].fill(0);
        Some(encode_ptr(seg_idx, start))
    }

    fn freelist_alloc(&mut self, size: usize) -> Option<Obj> {
        for s in 0..self.seg_count {
            if let Some(obj) = self.freelist_alloc_in_segment(s, size) {
                return Some(obj);
            }
        }
        None
    }

    fn freelist_alloc_in_segment(&mut self, seg_idx: usize, size: usize) -> Option<Obj> {
        let mut free = self.segments[seg_idx].free_head;
        let mut prev: Obj = 0;
        while !is_null(free) {
            let avail = self.get_size(free);
            if avail >= size {
                let newsize = avail - size;
                let next = self.get_fl_next(free);
                if newsize == 0 {
                    // exact fit: unlink
                    if is_null(prev) {
                        self.segments[seg_idx].free_head = next;
                    } else {
                        self.set_fl_next(prev, next);
                    }
                    self.zero_object_data(free, avail);
                    return Some(free);
                }
                // split: carve new object from the tail of the free block
                let (s, o) = Self::decode(free);
                let new_off = o + newsize;
                let new_obj = encode_ptr(s, new_off);
                self.set_size(free, newsize);
                self.set_size(new_obj, size);
                self.zero_object_data(new_obj, size);
                if self.show_abandons && newsize <= 2 {
                    println!("abandon small free block seg {} off {} size {}", s, o, newsize);
                }
                return Some(new_obj);
            }
            prev = free;
            free = self.get_fl_next(free);
        }
        None
    }

    fn zero_object_data(&mut self, obj: Obj, total_slots: usize) {
        let (s, o) = Self::decode(obj);
        let base = o + OBJECT_HEADER_SLOTS;
        let count = total_slots - OBJECT_HEADER_SLOTS;
        self.segments[s].mem[base..base + count].fill(0);
    }

    // -----------------------------------------------------------------------
    // GC: mark
    // -----------------------------------------------------------------------

    fn mark_object(&mut self, obj: Obj) {
        let (s, o) = Self::decode(obj);
        self.segments[s].mark(o);
    }

    fn is_marked(&self, obj: Obj) -> bool {
        let (s, o) = Self::decode(obj);
        self.segments[s].is_marked(o)
    }

    fn clear_mark_bits(&mut self) {
        for seg in self.segments.iter_mut() {
            seg.clear_marks();
        }
    }

    fn mark_and_scan(&mut self, root: Obj) {
        if is_null(root) || self.is_marked(root) {
            return;
        }
        self.mark_stack.push(root);
        while let Some(object) = self.mark_stack.pop() {
            if is_null(object) || self.is_marked(object) {
                continue;
            }
            self.mark_object(object);
            let slots: Obj = self.get_size(object);
            let (s, o) = Self::decode(object);
            for i in OBJECT_HEADER_SLOTS..slots {
                let child = self.segments[s].read(o + i);
                if !is_null(child) && !self.is_marked(child) {
                    self.mark_stack.push(child);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // GC: sweep
    // -----------------------------------------------------------------------

    fn sweep(&mut self) {
        self.lastgc_free_mem = 0;
        self.lastgc_live_mem = 0;

        for seg_idx in 0..self.seg_count {
            self.sweep_segment(seg_idx);
        }
        self.clear_mark_bits();

        if self.show_free_list {
            self.print_freelist();
        }
        if self.show_heap_map {
            self.print_heap();
        }
    }

    fn sweep_segment(&mut self, seg_idx: usize) {
        let upper = if self.segments[seg_idx].bump_exhausted {
            SEGMENT_SLOTS
        } else {
            self.segments[seg_idx].bump
        };

        self.segments[seg_idx].free_head = 0;
        let mut fl_tail: Obj = 0;
        let mut scan_off: usize = 1;

        while scan_off < upper {
            let obj = encode_ptr(seg_idx, scan_off);
            let size = self.get_size(obj);
            if size == 0 {
                break;
            }
            if self.is_marked(obj) {
                self.lastgc_live_mem += size;
            } else {
                self.lastgc_free_mem += size;
                if is_null(fl_tail) {
                    self.segments[seg_idx].free_head = obj;
                    self.set_fl_next(obj, 0);
                    fl_tail = obj;
                } else {
                    // coalesce if adjacent
                    if self.next_object_in_segment(fl_tail) == obj {
                        let combined = self.get_size(fl_tail) + size;
                        self.set_size(fl_tail, combined);
                    } else {
                        self.set_fl_next(fl_tail, obj);
                        self.set_fl_next(obj, 0);
                        fl_tail = obj;
                    }
                }
            }
            scan_off += size;
        }
    }

    // -----------------------------------------------------------------------
    // Debug printing
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Compaction: merge sparse segments and free empty ones
    // -----------------------------------------------------------------------

    /// Merge live objects from sparse segments into other segments, then
    /// drop empty segments.  Returns the number of segments freed.
    ///
    /// Call after `gc()` so free lists are up-to-date.
    pub fn compact(&mut self) -> usize {
        let dbg = self.show_compact;
        if dbg { println!("[compact] start: {} segments", self.seg_count); }

        // --- Step 1: compute live and available slots per segment -----------
        let mut live = vec![0usize; self.seg_count];
        let mut avail = vec![0usize; self.seg_count];
        for s in 0..self.seg_count {
            let seg = &self.segments[s];
            let upper = if seg.bump_exhausted { SEGMENT_SLOTS } else { seg.bump };
            let allocated = upper.saturating_sub(1);
            let mut free_slots = 0usize;
            let mut f = seg.free_head;
            while !is_null(f) {
                let size = header_size(self.segments[s].read(ptr_offset(f)));
                free_slots += size;
                f = self.segments[s].read(ptr_offset(f) + 1);
            }
            let bump_space = if seg.bump_exhausted { 0 } else { SEGMENT_SLOTS - seg.bump };
            live[s] = allocated - free_slots;
            avail[s] = free_slots + bump_space;
            if dbg {
                println!("[compact] seg {:3}: upper={} allocated={} free_list={} bump_free={} => live={} avail={}",
                    s, upper, allocated, free_slots, bump_space, live[s], avail[s]);
            }
        }

        // --- Step 2: greedy donor selection ---------------------------------
        let mut order: Vec<usize> = (0..self.seg_count).collect();
        order.sort_by_key(|&s| live[s]);

        let mut is_donor = vec![false; self.seg_count];

        for &s in &order {
            if live[s] == 0 {
                is_donor[s] = true;
                if dbg { println!("[compact] seg {:3}: donor (empty)", s); }
                continue;
            }
            let needed = live[s];
            let target = (0..self.seg_count)
                .find(|&t| t != s && !is_donor[t] && avail[t] >= needed);
            if let Some(t) = target {
                is_donor[s] = true;
                avail[t] -= needed;
                if dbg { println!("[compact] seg {:3}: donor (live={} fits into seg {})", s, needed, t); }
            } else {
                if dbg { println!("[compact] seg {:3}: kept   (live={} no target with avail>={})", s, needed, needed); }
            }
        }

        let donors: Vec<usize> = (0..self.seg_count).filter(|&s| is_donor[s]).collect();
        if donors.is_empty() {
            if dbg { println!("[compact] no donors found, nothing to do"); }
            return 0;
        }
        if dbg { println!("[compact] {} donor(s): {:?}", donors.len(), donors); }

        // --- Step 3: evacuate live objects from donor segments --------------
        let mut relocs: Vec<(Obj, Obj)> = Vec::new();

        for &donor in &donors {
            let free_offsets = self.collect_free_offsets(donor);
            if dbg { println!("[compact] evacuating seg {}: {} free offsets", donor, free_offsets.len()); }
            let upper = if self.segments[donor].bump_exhausted {
                SEGMENT_SLOTS
            } else {
                self.segments[donor].bump
            };
            let mut scan_off = 1usize;
            while scan_off < upper {
                let old_obj = encode_ptr(donor, scan_off);
                let size = self.get_size(old_obj);
                if size == 0 {
                    if dbg { println!("[compact]   seg {} off {}: size==0, stopping scan", donor, scan_off); }
                    break;
                }
                if free_offsets.binary_search(&scan_off).is_err() {
                    let new_obj = self.alloc_in_non_donor(size, &is_donor)
                        .expect("compact: ran out of space during evacuation");
                    let (new_s, new_o) = Self::decode(new_obj);
                    if dbg {
                        println!("[compact]   move live obj seg {} off {} size {} -> seg {} off {}",
                            donor, scan_off, size, new_s, new_o);
                    }
                    for i in OBJECT_HEADER_SLOTS..size {
                        let val = self.segments[donor].read(scan_off + i);
                        self.segments[new_s].write(new_o + i, val);
                    }
                    relocs.push((old_obj, new_obj));
                } else {
                    if dbg { println!("[compact]   seg {} off {} size {}: free, skipping", donor, scan_off, size); }
                }
                scan_off += size;
            }
        }
        if dbg { println!("[compact] step 3 done: {} relocations", relocs.len()); }

        // --- Step 4: apply relocations (object moves) -----------------------
        if dbg { println!("[compact] step 4: applying {} relocs to non-donor segments + roots", relocs.len()); }
        Self::apply_relocs_to_segments(&mut self.segments, self.seg_count, &is_donor, &relocs);
        let mut roots_updated = 0usize;
        for root in self.roots.iter_mut() {
            if let Some(new) = Self::lookup_reloc(&relocs, *root) {
                if dbg { println!("[compact]   root {:x} -> {:x}", *root, new); }
                *root = new;
                roots_updated += 1;
            }
        }
        if dbg { println!("[compact] step 4 done: {} root(s) updated", roots_updated); }

        // --- Step 5: compact the segments Vec, reindex all pointers ---------
        if dbg { println!("[compact] step 5: rebuilding segments vec, removing {} donor(s)", donors.len()); }
        let mut old_to_new: Vec<Option<usize>> = vec![None; self.seg_count];
        let mut new_segments: Vec<Segment> = Vec::with_capacity(self.seg_count - donors.len());
        for s in 0..self.seg_count {
            if !is_donor[s] {
                old_to_new[s] = Some(new_segments.len());
                if dbg { println!("[compact]   seg {} -> new idx {}", s, new_segments.len()); }
                let seg = std::mem::replace(&mut self.segments[s], Segment::new());
                new_segments.push(seg);
            } else {
                if dbg { println!("[compact]   seg {} dropped (donor)", s); }
            }
        }
        let freed = donors.len();
        self.segments = new_segments;
        self.seg_count = self.segments.len();
        self.current_seg = self.seg_count.saturating_sub(1);
        if dbg { println!("[compact] step 5 done: {} segments remain, current_seg={}", self.seg_count, self.current_seg); }

        if dbg { println!("[compact] step 6: reindexing all segment pointers"); }
        Self::reindex_all(&mut self.segments, self.seg_count, &mut self.roots, &old_to_new, dbg);
        if dbg { println!("[compact] done: freed {} segment(s), {} remain", freed, self.seg_count); }

        freed
    }

    /// Sorted list of free-block start offsets within a segment.
    fn collect_free_offsets(&self, seg_idx: usize) -> Vec<usize> {
        let mut offsets = Vec::new();
        let mut f = self.segments[seg_idx].free_head;
        while !is_null(f) {
            offsets.push(ptr_offset(f));
            f = self.segments[seg_idx].read(ptr_offset(f) + 1);
        }
        offsets.sort_unstable();
        offsets
    }

    /// Allocate `size` slots (including header) in any non-donor segment,
    /// using free-list first then bump.
    fn alloc_in_non_donor(&mut self, size: usize, is_donor: &[bool]) -> Option<Obj> {
        let dbg = self.show_compact;
        // free-list pass
        for s in 0..self.seg_count {
            if is_donor[s] { continue; }
            if let Some(obj) = self.freelist_alloc_in_segment(s, size) {
                if dbg { println!("[compact/alloc]   freelist seg {} -> {:x}", s, obj); }
                return Some(obj);
            }
        }
        // bump-space pass
        for s in 0..self.seg_count {
            if is_donor[s] { continue; }
            if !self.segments[s].bump_exhausted && self.segments[s].bump + size <= SEGMENT_SLOTS {
                let start = self.segments[s].bump;
                self.segments[s].bump += size;
                if self.segments[s].bump >= SEGMENT_SLOTS {
                    self.segments[s].bump_exhausted = true;
                }
                let hdr = encode_header(s, size);
                self.segments[s].mem[start] = hdr;
                self.segments[s].mem[start + OBJECT_HEADER_SLOTS..start + size].fill(0);
                let obj = encode_ptr(s, start);
                if dbg { println!("[compact/alloc]   bump seg {} off {} -> {:x}", s, start, obj); }
                return Some(obj);
            }
        }
        if dbg { println!("[compact/alloc]   FAILED for size {}", size); }
        None
    }

    fn lookup_reloc(relocs: &[(Obj, Obj)], ptr: Obj) -> Option<Obj> {
        relocs.iter().find(|&&(old, _)| old == ptr).map(|&(_, new)| new)
    }

    /// Rewrite pointer-valued data slots in all non-donor segments using the
    /// relocation table (object-move phase).
    fn apply_relocs_to_segments(
        segments: &mut Vec<Segment>,
        seg_count: usize,
        is_donor: &[bool],
        relocs: &[(Obj, Obj)],
    ) {
        for s in 0..seg_count {
            if is_donor[s] { continue; }
            let upper = if segments[s].bump_exhausted { SEGMENT_SLOTS } else { segments[s].bump };
            let mut off = 1usize;
            while off < upper {
                let size = header_size(segments[s].read(off));
                if size == 0 { break; }
                for i in OBJECT_HEADER_SLOTS..size {
                    let slot = segments[s].read(off + i);
                    if let Some(new) = Self::lookup_reloc(relocs, slot) {
                        segments[s].write(off + i, new);
                    }
                }
                off += size;
            }
        }
    }

    /// After the segments Vec has been compacted, rewrite the segment-index
    /// bits in every header, data slot, free_head, fl_next, and root using
    /// the old → new segment index mapping.
    fn reindex_all(
        segments: &mut Vec<Segment>,
        seg_count: usize,
        roots: &mut Vec<Obj>,
        old_to_new: &[Option<usize>],
        dbg: bool,
    ) {
        #[inline]
        fn remap(ptr: Obj, old_to_new: &[Option<usize>]) -> Obj {
            if is_null(ptr) { return 0; }
            let s = ptr_segment(ptr);
            let o = ptr_offset(ptr);
            match old_to_new.get(s).copied().flatten() {
                Some(ns) => encode_ptr(ns, o),
                None => ptr, // donor remnant — shouldn't be reachable
            }
        }

        let mut slots_rewritten = 0usize;
        let mut heads_rewritten = 0usize;

        for s in 0..seg_count {
            let old_fh = segments[s].free_head;
            let new_fh = remap(old_fh, old_to_new);
            if new_fh != old_fh {
                if dbg { println!("[compact/reindex] seg {} free_head {:x} -> {:x}", s, old_fh, new_fh); }
                segments[s].free_head = new_fh;
                heads_rewritten += 1;
            }

            let upper = if segments[s].bump_exhausted { SEGMENT_SLOTS } else { segments[s].bump };
            let mut off = 1usize;
            while off < upper {
                let raw = segments[s].read(off);
                let size = header_size(raw);
                if size == 0 {
                    if dbg { println!("[compact/reindex] seg {} off {}: size==0, stopping", s, off); }
                    break;
                }
                // Rewrite header segment bits to reflect new index s
                segments[s].write(off, encode_header(s, size));
                // Rewrite data slots (includes fl_next for free blocks at slot 1)
                for i in OBJECT_HEADER_SLOTS..size {
                    let slot = segments[s].read(off + i);
                    if !is_null(slot) {
                        let new_slot = remap(slot, old_to_new);
                        if new_slot != slot {
                            if dbg {
                                println!("[compact/reindex]   seg {} off {} slot {} {:x} -> {:x}",
                                    s, off, i, slot, new_slot);
                            }
                            segments[s].write(off + i, new_slot);
                            slots_rewritten += 1;
                        }
                    }
                }
                off += size;
            }
        }

        let mut roots_reindexed = 0usize;
        for root in roots.iter_mut() {
            let new = remap(*root, old_to_new);
            if new != *root {
                if dbg { println!("[compact/reindex] root {:x} -> {:x}", *root, new); }
                *root = new;
                roots_reindexed += 1;
            }
        }

        if dbg {
            println!("[compact/reindex] done: {} head(s), {} slot(s), {} root(s) rewritten",
                heads_rewritten, slots_rewritten, roots_reindexed);
        }
    }

    fn print_heap(&self) {
        println!("\n--- heap map ({} segments) ---", self.seg_count);
        for seg_idx in 0..self.seg_count {
            let seg = &self.segments[seg_idx];
            let upper = if seg.bump_exhausted {
                SEGMENT_SLOTS
            } else {
                seg.bump
            };
            print!("seg {:3}: [", seg_idx);
            let mut scan_off = 1usize;
            let mut col = 0usize;
            while scan_off < upper {
                let obj = encode_ptr(seg_idx, scan_off);
                let size = self.get_size(obj);
                if size == 0 {
                    break;
                }
                let is_free = {
                    let mut f = seg.free_head;
                    let mut found = false;
                    while !is_null(f) {
                        if ptr_offset(f) == scan_off {
                            found = true;
                            break;
                        }
                        f = self.get_fl_next(f);
                    }
                    found
                };
                let ch = if is_free { 'x' } else { '.' };
                for _ in 0..size / 2 {
                    print!("{}", ch);
                    col += 1;
                    if col % 120 == 0 {
                        println!();
                    }
                }
                scan_off += size;
            }
            println!("]");
        }
        self.print_gc_stats();
    }
}
