//! Core Wasm JSON ABI 的 filter guest 模組。
//!
//! 本模組接收結構化 entries 的 JSON request，標記 H1/H2，在其間套用 Warn 以上保留
//! 規則，再將逐筆 filter results 序列化為 JSON response 交還 host。

use core_wasm_json_common::{AbiError, AbiResponse, EntriesRequest, FilterOk};

#[unsafe(no_mangle)]
/// 在 guest 線性記憶體中配置 host 寫入 JSON request 所需的 buffer。
pub extern "C" fn alloc(len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let bytes = vec![0_u8; len as usize].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8 as u32
}

#[unsafe(no_mangle)]
/// 釋放先前由 guest 配置的 ABI buffer。
pub extern "C" fn dealloc(ptr: u32, len: u32) {
    unsafe { core_wasm_json_common::dealloc_bytes(ptr, len) };
}

#[unsafe(no_mangle)]
/// 解碼 entries request、產生保留判定，並回傳封裝 response 指標與長度的 `u64`。
pub extern "C" fn process(ptr: u32, len: u32) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let request: Result<EntriesRequest, _> = serde_json::from_slice(input);

    core_wasm_json_common::time_sign();
    let response = match request {
        Ok(request) => AbiResponse::Ok {
            ok: FilterOk {
                results: core_wasm_json_common::filter_entries(&request.entries),
            },
        },
        Err(error) => AbiResponse::Err {
            err: AbiError {
                kind: "invalid-abi-json".to_string(),
                message: error.to_string(),
            },
        },
    };
    core_wasm_json_common::time_sign();

    let output = serde_json::to_vec(&response).expect("serialize filter response");
    core_wasm_json_common::leak_bytes(output)
}
