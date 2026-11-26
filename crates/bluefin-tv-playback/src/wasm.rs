//! A bounded, dependency-free JS ABI. Host owns the input allocation until free;
//! output remains valid until the next apply. The package includes its JS host.
use std::cell::RefCell;
thread_local! { static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) }; }
#[unsafe(no_mangle)]
pub extern "C" fn tv_alloc(len: u32) -> *mut u8 {
    if len > 262144 {
        return std::ptr::null_mut();
    }
    let data = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(data) as *mut u8
}
/// # Safety
/// The pointer and length must be the matching live allocation from tv_alloc.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tv_free(ptr: *mut u8, len: u32) {
    if !ptr.is_null() && len <= 262144 {
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                ptr,
                len as usize,
            )));
        }
    }
}
/// # Safety
/// Input must name len readable bytes in the live tv_alloc allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tv_apply(ptr: *const u8, len: u32) -> *const u8 {
    let result = if ptr.is_null() || len > 262144 {
        serde_json::json!({"error":"input limit"})
    } else {
        match crate::client::apply(unsafe { std::slice::from_raw_parts(ptr, len as usize) }) {
            Ok(value) => serde_json::json!({"value":value}),
            Err(error) => serde_json::json!({"error":error}),
        }
    };
    OUTPUT.with(|output| {
        let mut output = output.borrow_mut();
        *output = serde_json::to_vec(&result).unwrap_or_default();
        output.as_ptr()
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn tv_result_len() -> u32 {
    OUTPUT.with(|o| o.borrow().len() as u32)
}
