# rustgc

Well, not technically a "real gc" but a memory allocator of slots within an array with GC being reclaiming of unused slots.

Slots are essentially indices, they could be forged hence, not so safe for rust itself but you can build a language on top which honours the memory rules. 

Next steps, adding tags for SmallInteger implementations (unboxed Smalltalk and JavaScript style) . 
