//! Namespaced plugin state: the host side of `kv_get`/`kv_put`/`kv_delete`/`kv_list`
//! (W53-6).
//!
//! Empty on purpose. `p53-runtime` pre-declares this module so the three lanes of
//! R3 each own one file and none of them edits `lib.rs`. W53-6 fills this in; the
//! guest-facing entries stay in `host_fns/kv.rs`, which is where the gate lives.
