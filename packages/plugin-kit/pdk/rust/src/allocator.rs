//! The guest allocator. wasm32 only.
//!
//! `marula_alloc` / `marula_free` are part of the ABI, so every plugin needs an
//! allocator whether or not it allocates. This one is a first-fit free list over
//! growing WASM memory: 16-byte blocks, a 16-byte header (total size, next free),
//! no coalescing.
//!
//! The shape follows from the invocation lifetime. A guest's allocations die
//! when the host drops the invocation's memory, so the free list only has to
//! survive one call's worth of churn (input buffer, reply buffer, the author's
//! `Vec`s). Coalescing would buy nothing a 64 MiB ceiling does not already
//! enforce, and it would cost the code a plugin author is expected to read.
//!
//! WASM is single-threaded here: one invocation per instance, one instance per
//! worker slot (ADR 0023 Decision 4). [`Heap`] therefore hands out interior
//! mutability without atomics, and the `Sync` promise below is the one place
//! that assumption is written down.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

/// Header size, and the alignment every block is handed out at.
const BLOCK: usize = 16;

struct State {
    /// First free block, `0` when the list is empty.
    free_head: usize,
    /// First never-allocated byte.
    brk: usize,
}

/// The heap state behind a `Sync` promise.
///
/// `Sync` is sound because wasm32 has no threads, and ADR 0023 Decision 4 fixes
/// one instance per invocation: two calls never share this state, and nothing in
/// the v1 ABI spawns a thread inside the instance.
struct Heap(UnsafeCell<State>);

// SAFETY: see the `Heap` doc comment — single-threaded instance, one at a time.
unsafe impl Sync for Heap {}

static STATE: Heap = Heap(UnsafeCell::new(State {
    free_head: 0,
    brk: 0,
}));

extern "C" {
    /// Linker-defined start of the heap, past the static data.
    static __heap_base: u8;
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Grows linear memory so `end` is addressable. `false` if the sandbox refuses.
fn grow_to(end: usize) -> bool {
    let have = core::arch::wasm32::memory_size(0) * 65_536;
    if end <= have {
        return true;
    }
    let pages = (end - have).div_ceil(65_536);
    core::arch::wasm32::memory_grow(0, pages) != usize::MAX
}

/// The allocator a plugin gets from [`crate::plugin!`].
pub struct GuestAllocator;

impl GuestAllocator {
    /// Returns a block to the free list. A no-op for a null pointer, so the host
    /// can free an empty reply without a special case.
    pub fn release(ptr: usize, _len: usize) {
        if ptr == 0 {
            return;
        }
        // SAFETY: `ptr` came from `take` and is being freed once; the header is
        // inside the block we handed out. Single-threaded, so no other writer.
        unsafe {
            let state = &mut *STATE.0.get();
            let header = (ptr - BLOCK) as *mut u32;
            *header.add(1) = state.free_head as u32;
            state.free_head = header as usize;
        }
    }

    /// First fit, then the bump pointer. Returns null when the block does not
    /// fit, which `GlobalAlloc`'s contract allows and which a plugin sees as an
    /// allocation failure rather than silent corruption.
    unsafe fn take(&self, layout: Layout) -> *mut u8 {
        if layout.align() > BLOCK {
            // Nothing the ABI or a plugin of this size asks for more than
            // 16-byte alignment; refusing beats handing out a misaligned block.
            return core::ptr::null_mut();
        }
        let size = layout.size().max(1);
        let needed = BLOCK + align_up(size, BLOCK);
        // SAFETY: single-threaded instance; see `Heap`.
        let state = unsafe { &mut *STATE.0.get() };
        if state.brk == 0 {
            state.brk = align_up(core::ptr::addr_of!(__heap_base) as usize, BLOCK);
        }

        let mut previous = 0usize;
        let mut current = state.free_head;
        while current != 0 {
            // SAFETY: every address on the free list is a header this allocator
            // wrote, inside linear memory it grew for that purpose.
            let header = current as *mut u32;
            let block_size = unsafe { *header } as usize;
            let next = unsafe { *header.add(1) } as usize;
            if block_size >= needed {
                if previous == 0 {
                    state.free_head = next;
                } else {
                    // SAFETY: `previous` is a live header, as above.
                    unsafe { *((previous as *mut u32).add(1)) = next as u32 };
                }
                return (current + BLOCK) as *mut u8;
            }
            previous = current;
            current = next;
        }

        let start = align_up(state.brk, BLOCK);
        let Some(end) = start.checked_add(needed) else {
            return core::ptr::null_mut();
        };
        if !grow_to(end) {
            return core::ptr::null_mut();
        }
        state.brk = end;
        // SAFETY: `start..end` is memory this allocator just made addressable.
        unsafe {
            let header = start as *mut u32;
            *header = needed as u32;
            *header.add(1) = 0;
        }
        (start + BLOCK) as *mut u8
    }
}

unsafe impl GlobalAlloc for GuestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { self.take(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        GuestAllocator::release(ptr as usize, 0);
    }
}

/// The `marula_alloc` path: a raw, 16-byte-aligned block of at least `size`.
pub fn arena_alloc(size: usize) -> *mut u8 {
    match Layout::from_size_align(size.max(1), BLOCK) {
        // SAFETY: the layout is valid and `take` enforces its own bounds.
        Ok(layout) => unsafe { GuestAllocator.take(layout) },
        Err(_) => core::ptr::null_mut(),
    }
}
