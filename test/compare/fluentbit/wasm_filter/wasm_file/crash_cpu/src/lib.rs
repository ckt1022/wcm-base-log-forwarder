// Scenario: CPU exhaustion — tight floating-point loop at ~100% CPU.
// Trigger: record JSON contains both "inject" key and value "cpu".
// Effect (observable): sink throughput drops to 0; CPU pegged at 100%;
//   container must be stopped externally (run_crash.sh enforces TEST_DURATION).
// black_box prevents the compiler from eliminating the loop body.
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
    if has_trigger(data, b"cpu") {
        let mut x: f64 = 1.0;
        loop {
            x = std::hint::black_box(f64::sqrt(x + 1.0));
        }
    }
    obj
}
