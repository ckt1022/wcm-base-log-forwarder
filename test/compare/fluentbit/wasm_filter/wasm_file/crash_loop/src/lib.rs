// Scenario: Infinite loop — blocks the Fluent Bit filter thread permanently.
// Trigger: record JSON contains both "inject" key and value "loop".
// Effect (observable): sink throughput drops to 0; CPU stays at 100%;
//   container must be stopped externally (run_crash.sh enforces TEST_DURATION).
//
// Return convention (Fluent Bit filter_wasm ABI):
//   NULL (0)       → discard record
//   non-NULL ptr   → pass record (return obj = unchanged pass-through)

fn has_trigger(data: &[u8], value: &[u8]) -> bool {
    // Require "inject" key to be present so normal records can't false-trigger.
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
    if has_trigger(data, b"loop") {
        loop {
            core::hint::spin_loop();
        }
    }
    obj // pass through: return original pointer so Fluent Bit keeps the record
}
