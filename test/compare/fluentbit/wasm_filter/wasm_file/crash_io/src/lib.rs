// Scenario: I/O blocking — calls WASI poll_oneoff with an infinite timeout,
//   modelling a plugin stuck waiting for an external I/O resource.
// Trigger: record JSON contains both "inject" key and value "io".
// Effect (observable): sink throughput drops to 0; CPU drops to ~0%
//   (blocked in WASI sleep, not spinning) — distinguishable from loop scenario.
// Fallback: if thread::sleep returns unexpectedly, spin_loop() keeps the
//   thread blocked so the observable effect is the same.
//
// Return convention: NULL (0) = discard; non-NULL ptr = keep record.

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
    let data = std::slice::from_raw_parts(obj, obj_len as usize);
    if has_trigger(data, b"io") {
        std::thread::sleep(std::time::Duration::from_secs(u64::MAX));
        loop {
            core::hint::spin_loop();
        }
    }
    obj
}
