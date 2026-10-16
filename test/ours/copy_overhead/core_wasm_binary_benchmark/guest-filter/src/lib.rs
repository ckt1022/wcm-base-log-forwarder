//! Core Wasm Direct Memory ABI 的 filter guest。
//!
//! 本模組以 borrowed entry views 開啟 compact request，在 H1/H2 間套用 Warn 以上保留
//! 規則，再將 filter results 編碼成 compact binary response。

#[unsafe(no_mangle)]
/// 配置 host 直接寫入 request 所需的 guest 線性記憶體。
pub extern "C" fn alloc(len: u32) -> u32 {
    core_wasm_binary_common::alloc_bytes(len)
}

#[unsafe(no_mangle)]
/// 釋放先前配置的 request 或 response ABI buffer。
pub extern "C" fn dealloc(ptr: u32, len: u32) {
    unsafe { core_wasm_binary_common::dealloc_bytes(ptr, len) };
}

#[unsafe(no_mangle)]
/// 開啟 zero-copy entry views、執行 filter，並回傳 packed response 指標與長度。
pub extern "C" fn process(ptr: u32, len: u32) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    // H1: the Direct Memory ABI has been opened as zero-copy typed entry views.
    // Filtering the views remains guest work and starts after this marker.
    let views = core_wasm_binary_common::entry_views(input);
    core_wasm_binary_common::time_sign();
    let result = views.and_then(core_wasm_binary_common::filter_views);
    core_wasm_binary_common::time_sign();

    match result {
        Ok(results) => core_wasm_binary_common::leak_bytes(
            core_wasm_binary_common::encode_filter_results(&results),
        ),
        Err(_) => 0,
    }
}
