//! The Marula plugin PDK, v1 — the guest half of ADR 0023.
//!
//! A plugin is a core WASM module with no WASI. It exports `marula_alloc`,
//! `marula_entry` and `marula_free`, and it imports one namespace, `marula`,
//! with nine entries. This crate supplies both halves so an author writes a
//! handler and nothing else:
//!
//! ```ignore
//! #![no_std]
//! extern crate alloc;
//!
//! use alloc::vec::Vec;
//! use marula_pdk::host;
//!
//! fn handle(input: &[u8]) -> Vec<u8> {
//!     host::log(host::Level::Info, "handling");
//!     let now = host::now();
//!     format!("{{\"seen\":{now}}}").into_bytes()
//! }
//!
//! marula_pdk::plugin!(handle);
//! ```
//!
//! What the macro provides: the three exports, a panic handler that traps
//! instead of looping (so a panic burns no fuel), and a per-invocation arena
//! allocator.
//!
//! ## The memory model, and why it is this small
//!
//! A guest's allocations live for one invocation: the host allocates the input
//! before [`marula_entry`] and reads the reply after it returns. So the
//! allocator keeps freed blocks on a free list and reuses them — it never
//! returns memory to the system, because WASM has no way to. A guest that grows
//! past [`LIMITS.instance_memory_bytes`] is killed by the sandbox, which is the
//! ceiling that matters.
//!
//! ## What the PDK does not do
//!
//! It does not parse JSON: a plugin that wants `serde_json` adds it itself, and
//! the reference plugins stay under a kilobyte. It does not hide a refusal: a
//! host call returns `Result<_, Refusal>` with the same strings and codes the
//! audit row carries.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod abi;

#[cfg(target_arch = "wasm32")]
mod allocator;
#[cfg(target_arch = "wasm32")]
pub mod host;

#[cfg(target_arch = "wasm32")]
pub use allocator::{arena_alloc, GuestAllocator};

pub use abi::{pack, unpack, Refusal, LIMITS};

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use allocator::arena_alloc as __arena_alloc;

/// Declares the v1 ABI for a plugin: the three exports, the panic handler and
/// the allocator. Call it once, at the root of the plugin crate, with the name
/// of a `fn(&[u8]) -> Vec<u8>`.
#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! plugin {
    ($handler:path) => {
        #[global_allocator]
        static __MARULA_ALLOCATOR: $crate::GuestAllocator = $crate::GuestAllocator;

        #[panic_handler]
        fn __marula_panic(_info: &core::panic::PanicInfo) -> ! {
            // A trap, not a loop: a panicking plugin should fail the invocation
            // immediately instead of burning its fuel budget.
            core::arch::wasm32::unreachable()
        }

        /// `marula_alloc(len) -> ptr`: the host places the invocation input and
        /// reads the reply through this.
        #[no_mangle]
        pub extern "C" fn marula_alloc(len: i32) -> i32 {
            $crate::__arena_alloc(len.max(1) as usize) as i32
        }

        /// `marula_free(ptr, len) -> ()`: returns a block to the guest free
        /// list. The arena is per-invocation, so a missed free is reclaimed by
        /// the sandbox's memory ceiling, not leaked into the next tenant.
        #[no_mangle]
        pub extern "C" fn marula_free(ptr: i32, len: i32) {
            $crate::GuestAllocator::release(ptr as usize, len.max(0) as usize);
        }

        /// `marula_entry(ptr, len) -> i64`: the packed `(ptr, len)` of the reply.
        #[no_mangle]
        pub extern "C" fn marula_entry(ptr: i32, len: i32) -> i64 {
            let input: &[u8] = if len <= 0 {
                &[]
            } else {
                // SAFETY: the host wrote `len` bytes at `ptr` through
                // `marula_alloc` before calling entry; the slice lives for the
                // duration of the call.
                unsafe { core::slice::from_raw_parts(ptr as usize as *const u8, len as usize) }
            };
            let output = $handler(input);
            let out_ptr = $crate::__arena_alloc(output.len().max(1));
            // SAFETY: `out_ptr` is a fresh, non-overlapping block of at least
            // `output.len()` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(output.as_ptr(), out_ptr, output.len());
            }
            $crate::pack(out_ptr as u32, output.len() as u32)
        }
    };
}
