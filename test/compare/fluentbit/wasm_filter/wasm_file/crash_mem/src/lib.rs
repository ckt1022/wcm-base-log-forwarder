// Scenario: Memory exhaustion — grows WASM linear memory by 1 page (64 KB)
//   every 32 records after trigger fires (~2 KB/record average, matching Lua).
//   Models a plugin that silently retains all log data.
//
// Trigger: record bytes contain both "inject" and "mem".
//   Once triggered, LEAK_ACTIVE stays true for all subsequent records
//   (same WASM instance reused across all filter calls).
//
// Effect (observable): container RSS grows ~20 MB/s (10 000 lps × 2 KB avg)
//   until Docker OOM-kills the process (exit 137).
//   run_crash.sh sets --memory=512m as the ceiling (~25 s to OOM).
//
// Why memory_grow instead of Box::leak:
//   Box::leak goes through Rust's dlmalloc allocator which calls memory.grow
//   internally. If WAMR's wasm_heap_size caps that path, dlmalloc panics/traps
//   and Fluent Bit swallows the trap — memory stays flat. Calling memory_grow
//   directly bypasses dlmalloc and is guaranteed to expand WASM linear memory.
//   Writing 0xFF to the new page forces physical page commitment → RSS grows.
//
// Return convention: NULL (0) = discard; non-NULL ptr = keep record.
// Records continue to flow (we return obj) so sink throughput stays up
// while memory grows — the OOM kill is the observable event.

use core::arch::wasm32;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static LEAK_ACTIVE: AtomicBool = AtomicBool::new(false);
static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

// Grow 1 page (64 KB) every 32 records → 64 KB / 32 = 2 KB per record average.
// Matches Lua's string.rep("X", 2048) leak rate.
const GROW_EVERY: usize = 32;

fn has_trigger(data: &[u8], value: &[u8]) -> bool {
    data.windows(6).any(|w| w == b"inject")
        && data.windows(value.len()).any(|w| w == value)
}

#[no_mangle]
pub unsafe extern "C" fn filter_func(
    _tag: *const u8,
    _tag_len: i32,
    _time_sec: u32,
    _time_nsec: u32,
    obj: *const u8,
    obj_len: i32,
) -> *const u8 {
    if obj.is_null() || obj_len <= 0 {
        return obj;
    }
    let data = core::slice::from_raw_parts(obj, obj_len as usize);

    if has_trigger(data, b"mem") {
        LEAK_ACTIVE.store(true, Ordering::Relaxed);
    }

    if LEAK_ACTIVE.load(Ordering::Relaxed) {
        let n = CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        if n % GROW_EVERY == 0 {
            // Grow WASM linear memory by 1 page (64 KB).
            // Returns previous page count on success, usize::MAX on failure.
            let prev = wasm32::memory_grow(0, 1);
            if prev != usize::MAX {
                // Touch the first byte of the new page to force physical
                // page commitment — without this the OS may keep it as a
                // zero-page mapping and Docker RSS won't increase.
                let new_page_start = (prev * 65536) as *mut u8;
                new_page_start.write_volatile(0xFF);
            }
        }
    }

    obj // records continue to flow; memory grows monotonically
}
