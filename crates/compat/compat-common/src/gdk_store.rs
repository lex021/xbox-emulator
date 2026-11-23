
use core::ffi::c_void;
use crate::gdk_user::{start_async, write_jump, xasync_complete, xasync_get_status, ProviderData, E_PENDING};

const S_OK: i32 = 0;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_ABORT: i32 = 0x8000_4004u32 as i32;

static CONTEXT_ANCHOR: u64 = 0x5853_544F_5245_4354;
static QUERY_ANCHOR: u64 = 0x5853_544F_5245_5152;
fn ctx() -> usize { &CONTEXT_ANCHOR as *const u64 as usize }
fn query() -> usize { &QUERY_ANCHOR as *const u64 as usize }

fn log(func: &str, detail: &str) {
    crate::gdk_user::log(func, detail);
}

unsafe extern "system" fn ok_provider(op: u32, data: *const ProviderData) -> i32 {
    if op == 1 {
        xasync_complete((*data).async_block, S_OK, 0);
        return E_PENDING;
    }
    S_OK
}
unsafe extern "system" fn cancelled_provider(op: u32, data: *const ProviderData) -> i32 {
    if op == 1 {
        xasync_complete((*data).async_block, E_ABORT, 0);
        return E_PENDING;
    }
    S_OK
}

unsafe extern "system" fn create_context(user: usize, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = ctx();
    log("XStoreCreateContext", &format!("user={user:#x} -> local empty store"));
    S_OK
}
unsafe extern "system" fn close_handle(_h: usize) {}

unsafe extern "system" fn updates_async(_ctx: usize, block: *mut c_void) -> i32 {
    log("XStoreQueryGameAndDlcPackageUpdatesAsync", "-> no updates");
    start_async(block, updates_async as usize, b"XStoreQueryGameAndDlcPackageUpdatesAsync\0", ok_provider)
}
unsafe extern "system" fn updates_count(block: *mut c_void, count: *mut u32) -> i32 {
    let hr = xasync_get_status(block);
    if hr >= 0 && !count.is_null() {
        *count = 0;
    }
    hr
}
unsafe extern "system" fn updates_result(block: *mut c_void, _count: u32, _out: *mut c_void) -> i32 {
    xasync_get_status(block)
}

unsafe extern "system" fn products_async(_ctx: usize, kinds: u32, _max: u32, block: *mut c_void) -> i32 {
    log("XStoreQuery(Associated|Entitled)ProductsAsync", &format!("kinds={kinds:#x} -> empty"));
    start_async(block, products_async as usize, b"XStoreQueryProductsAsync\0", ok_provider)
}
unsafe extern "system" fn products_by_id_async(_ctx: usize, kinds: u32, _ids: *const *const u8, n: usize, _filters: *const *const u8, _nf: usize, block: *mut c_void) -> i32 {
    log("XStoreQueryProductsAsync", &format!("kinds={kinds:#x} ids={n} -> empty"));
    start_async(block, products_async as usize, b"XStoreQueryProductsAsync\0", ok_provider)
}
unsafe extern "system" fn next_page_async(_q: usize, block: *mut c_void) -> i32 {
    start_async(block, products_async as usize, b"XStoreProductsQueryNextPageAsync\0", ok_provider)
}
unsafe extern "system" fn products_result(block: *mut c_void, out: *mut usize) -> i32 {
    let hr = xasync_get_status(block);
    if hr >= 0 && !out.is_null() {
        *out = query();
    }
    hr
}
unsafe extern "system" fn enumerate(_q: usize, _ctx: *mut c_void, _cb: *const c_void) -> i32 {
    S_OK
}
unsafe extern "system" fn has_more_pages(_q: usize) -> bool {
    false
}

unsafe extern "system" fn consumable_async(_ctx: usize, _id: *const u8, block: *mut c_void) -> i32 {
    log("XStoreQueryConsumableBalanceRemainingAsync", "-> balance 0");
    start_async(block, consumable_async as usize, b"XStoreQueryConsumableBalanceRemainingAsync\0", ok_provider)
}
unsafe extern "system" fn consumable_result(block: *mut c_void, out: *mut u32) -> i32 {
    let hr = xasync_get_status(block);
    if hr >= 0 && !out.is_null() {
        *out = 0;
    }
    hr
}

unsafe extern "system" fn purchase_ui_async(_ctx: usize, _id: *const u8, _name: *const u8, _json: *const u8, block: *mut c_void) -> i32 {
    log("XStoreShowPurchaseUIAsync", "-> cancelled (no store)");
    start_async(block, purchase_ui_async as usize, b"XStoreShowPurchaseUIAsync\0", cancelled_provider)
}
unsafe extern "system" fn redeem_ui_async(_ctx: usize, _token: *const u8, _ids: *const *const u8, _n: usize, _csv: bool, block: *mut c_void) -> i32 {
    log("XStoreShowRedeemTokenUIAsync", "-> cancelled (no store)");
    start_async(block, redeem_ui_async as usize, b"XStoreShowRedeemTokenUIAsync\0", cancelled_provider)
}
unsafe extern "system" fn ui_result(block: *mut c_void) -> i32 {
    xasync_get_status(block)
}

pub(crate) unsafe fn install(base: *mut u8, rvas: &[usize]) {
    let table: [(&str, usize); 22] = [
        ("XStoreCloseContextHandle", close_handle as usize),
        ("XStoreCloseProductsQueryHandle", close_handle as usize),
        ("XStoreCreateContext", create_context as usize),
        ("XStoreEnumerateProductsQuery", enumerate as usize),
        ("XStoreProductsQueryHasMorePages", has_more_pages as usize),
        ("XStoreProductsQueryNextPageAsync", next_page_async as usize),
        ("XStoreProductsQueryNextPageResult", products_result as usize),
        ("XStoreQueryAssociatedProductsAsync", products_async as usize),
        ("XStoreQueryAssociatedProductsResult", products_result as usize),
        ("XStoreQueryConsumableBalanceRemainingAsync", consumable_async as usize),
        ("XStoreQueryConsumableBalanceRemainingResult", consumable_result as usize),
        ("XStoreQueryEntitledProductsAsync", products_async as usize),
        ("XStoreQueryEntitledProductsResult", products_result as usize),
        ("XStoreQueryGameAndDlcPackageUpdatesAsync", updates_async as usize),
        ("XStoreQueryGameAndDlcPackageUpdatesResult", updates_result as usize),
        ("XStoreQueryGameAndDlcPackageUpdatesResultCount", updates_count as usize),
        ("XStoreQueryProductsAsync", products_by_id_async as usize),
        ("XStoreQueryProductsResult", products_result as usize),
        ("XStoreShowPurchaseUIAsync", purchase_ui_async as usize),
        ("XStoreShowPurchaseUIResult", ui_result as usize),
        ("XStoreShowRedeemTokenUIAsync", redeem_ui_async as usize),
        ("XStoreShowRedeemTokenUIResult", ui_result as usize),
    ];
    let mut ok = 0;
    for ((_, target), rva) in table.iter().zip(rvas) {
        if write_jump(base.add(*rva), *target) {
            ok += 1;
        }
    }
    crate::record_raw("gdk_store", "install", &format!("local empty store installed ({ok}/22 XStore wrappers redirected)"));
}

pub const XSTORE_NAMES: [&str; 22] = [
    "XStoreCloseContextHandle",
    "XStoreCloseProductsQueryHandle",
    "XStoreCreateContext",
    "XStoreEnumerateProductsQuery",
    "XStoreProductsQueryHasMorePages",
    "XStoreProductsQueryNextPageAsync",
    "XStoreProductsQueryNextPageResult",
    "XStoreQueryAssociatedProductsAsync",
    "XStoreQueryAssociatedProductsResult",
    "XStoreQueryConsumableBalanceRemainingAsync",
    "XStoreQueryConsumableBalanceRemainingResult",
    "XStoreQueryEntitledProductsAsync",
    "XStoreQueryEntitledProductsResult",
    "XStoreQueryGameAndDlcPackageUpdatesAsync",
    "XStoreQueryGameAndDlcPackageUpdatesResult",
    "XStoreQueryGameAndDlcPackageUpdatesResultCount",
    "XStoreQueryProductsAsync",
    "XStoreQueryProductsResult",
    "XStoreShowPurchaseUIAsync",
    "XStoreShowPurchaseUIResult",
    "XStoreShowRedeemTokenUIAsync",
    "XStoreShowRedeemTokenUIResult",
];
