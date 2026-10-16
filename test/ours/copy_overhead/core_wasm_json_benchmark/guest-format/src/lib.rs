//! Core Wasm JSON ABI 的 format guest 模組。
//!
//! 本模組接收結構化 entries 的 JSON request，在 H1/H2 間產生 length-prefixed 扁平
//! JSON frames，並於 H2 後將二進位結果以 Base64 包入 JSON response 交還 host。

use base64::Engine;
use core_wasm_json_common::{AbiError, AbiResponse, EntriesRequest, FormatOk};

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
/// 解碼 entries、格式化 frames，並回傳封裝 JSON response 指標與長度的 `u64`。
pub extern "C" fn process(ptr: u32, len: u32) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let request: Result<EntriesRequest, _> = serde_json::from_slice(input);

    core_wasm_json_common::time_sign();
    let response = match request {
        Ok(request) => {
            let framed = core_wasm_json_common::format_entries(&request.entries);
            core_wasm_json_common::time_sign();
            AbiResponse::Ok {
                ok: FormatOk {
                    framed_output_base64: base64::engine::general_purpose::STANDARD.encode(framed),
                },
            }
        }
        Err(error) => {
            core_wasm_json_common::time_sign();
            AbiResponse::Err {
                err: AbiError {
                    kind: "invalid-abi-json".to_string(),
                    message: error.to_string(),
                },
            }
        }
    };

    let output = serde_json::to_vec(&response).expect("serialize format response");
    core_wasm_json_common::leak_bytes(output)
}
