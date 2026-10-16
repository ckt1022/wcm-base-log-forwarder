//! Core Wasm Direct Memory ABI 的 parse guest。
//!
//! 本模組以 borrowed record views 開啟 host 寫入的 compact request，在 H1/H2 間解析
//! 原始 JSON logs，再將 parsed entries 編碼成 compact binary response。

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
/// 開啟 zero-copy record views、執行 parse，並回傳 packed response 指標與長度。
pub extern "C" fn process(ptr: u32, len: u32) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    // H1: the Direct Memory ABI has been opened as a zero-copy record view.
    // JSON parsing remains guest work and therefore starts after this marker.
    let views = core_wasm_binary_common::record_views(input);
    core_wasm_binary_common::time_sign();
    let result = views.and_then(core_wasm_binary_common::parse_views);
    core_wasm_binary_common::time_sign();

    match result {
        Ok(entries) => core_wasm_binary_common::leak_bytes(
            core_wasm_binary_common::encode_parsed_entries(&entries),
        ),
        Err(_) => 0,
    }
}
