use super::events::Registry;
use super::win::Dialog;
use super::xasync::{self, Payload};
use super::{cstr, data_root, log, title, Guid, Iface, This, E_ABORT, E_INVALIDARG, E_NOT_SUFFICIENT_BUFFER, S_OK};
use core::ffi::c_void;
use serde_json::{json, Value};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const CLSID: Guid = Guid::parse("0dd112ac-7c24-448c-b92b-3960fb5bd30c");
pub const IID: Guid = Guid::parse("de3dbdd4-0b37-4bdb-a10e-acf3a354d06a");
pub const IID_V2: Guid = Guid::parse("2d42fea5-e71d-4b76-97cd-c50afbb3ae5d");
pub const IID_V3: Guid = Guid::parse("60b09f4e-1b85-45b1-826c-169118e230e1");

pub const E_GAMESTORE_LICENSE_ACTION_NOT_APPLICABLE_TO_PRODUCT: i32 = 0x8924_5300u32 as i32;
pub const E_GAMESTORE_NETWORK_ERROR: i32 = 0x8924_5301u32 as i32;
pub const E_GAMESTORE_SERVER_ERROR: i32 = 0x8924_5302u32 as i32;
pub const E_GAMESTORE_INSUFFICIENT_QUANTITY: i32 = 0x8924_5303u32 as i32;
pub const E_GAMESTORE_ALREADY_PURCHASED: i32 = 0x8924_5304u32 as i32;
pub const E_GAMESTORE_LICENSE_ACTION_THROTTLED: i32 = 0x8924_5305u32 as i32;
pub const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;

static CONTEXT_ANCHOR: u64 = 0x5853_544F_5245_4354;

fn ctx() -> usize {
    &CONTEXT_ANCHOR as *const u64 as usize
}

pub fn store_dir() -> PathBuf {
    data_root().join("store").join(super::sanitize(&title().key))
}

fn config_text(tag: &str) -> Option<String> {
    let dir = title().dir.as_ref()?;
    let t = std::fs::read_to_string(dir.join("MicrosoftGame.config")).ok()?;
    let open = format!("<{tag}>");
    let a = t.find(&open)? + open.len();
    let b = t[a..].find(&format!("</{tag}>"))?;
    Some(t[a..a + b].trim().to_string())
}

pub fn game_store_id() -> String {
    super::setting("store_id").or_else(|| config_text("StoreId")).unwrap_or_else(|| "9NBLGGH4R315".into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Product {
    pub store_id: String,
    pub title: String,
    pub description: String,
    pub kind: u32,
    pub price: f32,
    pub formatted: String,
    pub currency: String,
    pub consumable: bool,
}

fn kind_of(s: &str) -> u32 {
    match s.to_ascii_lowercase().as_str() {
        "consumable" => 0x1,
        "durable" => 0x2,
        "game" => 0x4,
        "pass" => 0x8,
        "unmanagedconsumable" => 0x10,
        _ => 0x2,
    }
}

pub fn catalog() -> Vec<Product> {
    let mut v = vec![Product {
        store_id: game_store_id(),
        title: title().display.clone(),
        description: String::new(),
        kind: 0x4,
        price: 0.0,
        formatted: "Owned".into(),
        currency: "USD".into(),
        consumable: false,
    }];
    let Ok(t) = std::fs::read_to_string(store_dir().join("catalog.json")) else { return v };
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&t) else { return v };
    for p in a {
        let Some(id) = p["storeId"].as_str() else { continue };
        let kind = kind_of(p["kind"].as_str().unwrap_or("durable"));
        let price = p["price"].as_f64().unwrap_or(0.0) as f32;
        v.push(Product {
            store_id: id.to_string(),
            title: p["title"].as_str().unwrap_or(id).to_string(),
            description: p["description"].as_str().unwrap_or("").to_string(),
            kind,
            price,
            formatted: p["formattedPrice"].as_str().map(|s| s.to_string()).unwrap_or_else(|| format!("${price:.2}")),
            currency: p["currency"].as_str().unwrap_or("USD").to_string(),
            consumable: kind & 0x11 != 0,
        });
    }
    v
}

fn state_lock() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

fn load_state() -> Value {
    std::fs::read_to_string(store_dir().join("owned.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({"owned": [], "balances": {}, "ratings": []}))
}

fn save_state(v: &Value) -> i32 {
    let d = store_dir();
    if let Err(e) = std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join("owned.json"), v.to_string())) {
        return super::win::hr_from_win32(e.raw_os_error().unwrap_or(29) as u32);
    }
    S_OK
}

pub fn owns(id: &str) -> bool {
    if id.eq_ignore_ascii_case(&game_store_id()) {
        return true;
    }
    let _g = state_lock().lock();
    load_state()["owned"].as_array().map(|a| a.iter().any(|x| x.as_str().map(|s| s.eq_ignore_ascii_case(id)).unwrap_or(false))).unwrap_or(false)
}

pub fn grant(id: &str, quantity: u32) -> i32 {
    let _g = state_lock().lock();
    let mut s = load_state();
    let consumable = catalog().iter().any(|p| p.store_id.eq_ignore_ascii_case(id) && p.consumable);
    if consumable {
        let cur = s["balances"][id].as_u64().unwrap_or(0);
        s["balances"][id] = json!(cur + quantity as u64);
    } else if let Some(a) = s["owned"].as_array_mut() {
        if !a.iter().any(|x| x.as_str() == Some(id)) {
            a.push(json!(id));
        }
    }
    save_state(&s)
}

pub fn balance(id: &str) -> u32 {
    let _g = state_lock().lock();
    load_state()["balances"][id].as_u64().unwrap_or(0) as u32
}

fn consume(id: &str, quantity: u32) -> Result<u32, i32> {
    let _g = state_lock().lock();
    let mut s = load_state();
    let cur = s["balances"][id].as_u64().unwrap_or(0) as u32;
    if cur < quantity {
        return Err(E_GAMESTORE_INSUFFICIENT_QUANTITY);
    }
    s["balances"][id] = json!(cur - quantity);
    let hr = save_state(&s);
    if hr < 0 { Err(hr) } else { Ok(cur - quantity) }
}

struct ProductQuery {
    items: Vec<Product>,
}

static OP_PRODUCTS: &[u8] = b"XStoreQueryProductsAsync\0";
static OP_UPDATES: &[u8] = b"XStoreQueryGameAndDlcPackageUpdatesAsync\0";
static OP_CONSUMABLE: &[u8] = b"XStoreQueryConsumableBalanceRemainingAsync\0";
static OP_UI: &[u8] = b"XStoreShowUIAsync\0";
static OP_LICENSE: &[u8] = b"XStoreAcquireLicenseAsync\0";
static OP_CAN: &[u8] = b"XStoreCanAcquireLicenseAsync\0";
static OP_GAME_LICENSE: &[u8] = b"XStoreQueryGameLicenseAsync\0";
static OP_ADDONS: &[u8] = b"XStoreQueryAddOnLicensesAsync\0";
static OP_FULFILL: &[u8] = b"XStoreReportConsumableFulfillmentAsync\0";
static OP_STRING: &[u8] = b"XStoreStringResultAsync\0";
static OP_PACKAGES: &[u8] = b"XStoreDownloadAndInstallPackagesAsync\0";
static OP_RATE: &[u8] = b"XStoreShowRateAndReviewUIAsync\0";

unsafe extern "system" fn create_context(_t: This, user: usize, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = ctx();
    log("XStoreCreateContext", &format!("user={user:#x} -> store {}", store_dir().display()));
    S_OK
}

unsafe extern "system" fn close_handle(_t: This, _h: usize) {}

fn query(items: Vec<Product>) -> Payload {
    Payload::value(Box::into_raw(Box::new(ProductQuery { items })) as usize)
}

unsafe extern "system" fn associated_async(_t: This, _ctx: usize, kinds: u32, _max: u32, block: *mut c_void) -> i32 {
    let items = catalog().into_iter().filter(|p| p.kind & kinds != 0 && p.kind != 0x4).collect();
    xasync::run(block, OP_PRODUCTS, query(items))
}

unsafe extern "system" fn entitled_async(_t: This, _ctx: usize, kinds: u32, _max: u32, block: *mut c_void) -> i32 {
    let items = catalog().into_iter().filter(|p| p.kind & kinds != 0 && (owns(&p.store_id) || (p.consumable && balance(&p.store_id) > 0))).collect();
    xasync::run(block, OP_PRODUCTS, query(items))
}

unsafe fn str_list(p: *const *const u8, n: usize) -> Vec<String> {
    if p.is_null() {
        return Vec::new();
    }
    (0..n).filter_map(|i| cstr(*p.add(i))).collect()
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn products_by_id_async(_t: This, _ctx: usize, kinds: u32, ids: *const *const u8, n: usize, _f: *const *const u8, _nf: usize, block: *mut c_void) -> i32 {
    let want = str_list(ids, n);
    let items = catalog().into_iter().filter(|p| p.kind & kinds != 0 && want.iter().any(|w| w.eq_ignore_ascii_case(&p.store_id))).collect();
    xasync::run(block, OP_PRODUCTS, query(items))
}

unsafe extern "system" fn current_game_async(_t: This, _ctx: usize, block: *mut c_void) -> i32 {
    let items = catalog().into_iter().filter(|p| p.kind == 0x4).collect();
    xasync::run(block, OP_PRODUCTS, query(items))
}

unsafe extern "system" fn for_package_async(_t: This, _ctx: usize, kinds: u32, package: *const u8, block: *mut c_void) -> i32 {
    let pkg = cstr(package).unwrap_or_default();
    let items = if pkg == super::package::package_id() { catalog().into_iter().filter(|p| p.kind == 0x4 && p.kind & kinds != 0).collect() } else { Vec::new() };
    xasync::run(block, OP_PRODUCTS, query(items))
}

unsafe extern "system" fn next_page_async(_t: This, _q: usize, block: *mut c_void) -> i32 {
    xasync::run(block, OP_PRODUCTS, query(Vec::new()))
}

unsafe extern "system" fn products_result(_t: This, block: *mut c_void, out: *mut usize) -> i32 {
    xasync::value(block, OP_PRODUCTS, out)
}

unsafe extern "system" fn close_query(_t: This, q: usize) {
    if q != 0 {
        drop(Box::from_raw(q as *mut ProductQuery));
    }
}

type ProductCb = unsafe extern "system" fn(*const u8, *mut c_void) -> bool;

fn put_fixed(b: &mut [u8], off: usize, cap: usize, s: &str) {
    let n = s.len().min(cap - 1);
    b[off..off + n].copy_from_slice(&s.as_bytes()[..n]);
}

fn put_ptr(b: &mut [u8], off: usize, p: *const i8) {
    b[off..off + 8].copy_from_slice(&(p as usize as u64).to_le_bytes());
}

fn write_price(b: &mut [u8], off: usize, p: &Product, currency: *const i8) {
    b[off..off + 4].copy_from_slice(&p.price.to_le_bytes());
    b[off + 4..off + 8].copy_from_slice(&p.price.to_le_bytes());
    put_ptr(b, off + 16, currency);
    put_fixed(b, off + 24, 16, &p.formatted);
    put_fixed(b, off + 40, 16, &p.formatted);
}

pub const PRODUCT_SIZE: usize = 208;
pub const SKU_SIZE: usize = 272;

unsafe fn with_product<R>(p: &Product, f: impl FnOnce(*const u8) -> R) -> R {
    let owned = owns(&p.store_id) || (p.consumable && balance(&p.store_id) > 0);
    let link = format!("https://www.microsoft.com/store/productId/{}", p.store_id);
    let s: Vec<CString> = [p.store_id.as_str(), p.title.as_str(), p.description.as_str(), super::package::user_locale().as_str(), "", link.as_str(), p.currency.as_str(), "0010"]
        .iter()
        .map(|x| CString::new(*x).unwrap_or_default())
        .collect();
    let mut sku = vec![0u8; SKU_SIZE];
    put_ptr(&mut sku, 0, s[7].as_ptr());
    put_ptr(&mut sku, 8, s[1].as_ptr());
    put_ptr(&mut sku, 16, s[2].as_ptr());
    put_ptr(&mut sku, 24, s[3].as_ptr());
    write_price(&mut sku, 32, p, s[6].as_ptr());
    sku[121] = owned as u8;
    if owned {
        sku[160..164].copy_from_slice(&if p.consumable { balance(&p.store_id) } else { 1 }.to_le_bytes());
    }
    let mut d = vec![0u8; PRODUCT_SIZE];
    for (i, off) in [0usize, 8, 16, 24, 32, 40].iter().enumerate() {
        put_ptr(&mut d, *off, s[i].as_ptr());
    }
    d[48..52].copy_from_slice(&p.kind.to_le_bytes());
    write_price(&mut d, 56, p, s[6].as_ptr());
    d[144] = 1;
    d[145] = owned as u8;
    d[160..164].copy_from_slice(&1u32.to_le_bytes());
    put_ptr(&mut d, 168, sku.as_ptr() as *const i8);
    f(d.as_ptr())
}

unsafe extern "system" fn enumerate(_t: This, q: usize, ctx: *mut c_void, cb: Option<ProductCb>) -> i32 {
    let (Some(q), Some(cb)) = ((q as *const ProductQuery).as_ref(), cb) else { return E_INVALIDARG };
    for p in &q.items {
        if !with_product(p, |b| cb(b, ctx)) {
            break;
        }
    }
    S_OK
}

unsafe extern "system" fn has_more_pages(_t: This, _q: usize) -> bool {
    false
}

unsafe extern "system" fn consumable_async(_t: This, _ctx: usize, id: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    xasync::run(block, OP_CONSUMABLE, Payload::value(balance(&id)))
}

unsafe extern "system" fn consumable_result(_t: This, block: *mut c_void, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp = [0u8; 4];
    let hr = xasync::bytes(block, OP_CONSUMABLE, tmp.len(), tmp.as_mut_ptr() as *mut c_void, core::ptr::null_mut());
    if hr >= 0 {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), out, 4);
    }
    hr
}

unsafe extern "system" fn fulfill_async(_t: This, _ctx: usize, id: *const u8, quantity: u32, _tracking: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    let p = match consume(&id, quantity) {
        Ok(left) => Payload::value(left),
        Err(hr) => Payload::status(hr),
    };
    xasync::run(block, OP_FULFILL, p)
}

unsafe extern "system" fn fulfill_result(_t: This, block: *mut c_void, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp = [0u8; 4];
    let hr = xasync::bytes(block, OP_FULFILL, tmp.len(), tmp.as_mut_ptr() as *mut c_void, core::ptr::null_mut());
    if hr >= 0 {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), out, 4);
    }
    hr
}

fn find(id: &str) -> Option<Product> {
    catalog().into_iter().find(|p| p.store_id.eq_ignore_ascii_case(id))
}

unsafe extern "system" fn purchase_ui_async(_t: This, _ctx: usize, id: *const u8, _name: *const u8, _json: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    xasync::run_with(block, OP_UI, Box::new(move |c| {
        let Some(p) = find(&id) else {
            super::win::show(Dialog { title: "Microsoft Store".into(), text: format!("This item isn't available.\n\n{id}"), buttons: vec!["OK".into()], cancel: Some(0), ..Default::default() }, Some(c.window.clone()));
            return Payload::status(E_NOT_FOUND);
        };
        if !p.consumable && owns(&p.store_id) {
            super::win::show(Dialog { title: "Microsoft Store".into(), text: format!("You already own {}.", p.title), buttons: vec!["OK".into()], cancel: Some(0), ..Default::default() }, Some(c.window.clone()));
            return Payload::status(E_GAMESTORE_ALREADY_PURCHASED);
        }
        let d = Dialog {
            title: "Microsoft Store".into(),
            text: format!("{}\n\n{}\n\nPrice: {}", p.title, p.description, p.formatted),
            buttons: vec!["Buy".into(), "Cancel".into()],
            default: 0,
            cancel: Some(1),
            ..Default::default()
        };
        let o = super::win::show(d, Some(c.window.clone()));
        if c.is_set() || o.button != Some(0) {
            return Payload::status(E_ABORT);
        }
        Payload::status(grant(&p.store_id, 1))
    }))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn redeem_ui_async(_t: This, _ctx: usize, token: *const u8, ids: *const *const u8, n: usize, _no_csv: bool, block: *mut c_void) -> i32 {
    let default = cstr(token).unwrap_or_default();
    let allowed = str_list(ids, n);
    xasync::run_with(block, OP_UI, Box::new(move |c| {
        let d = Dialog {
            title: "Redeem a code".into(),
            text: "Enter the 25-character code.".into(),
            edit: Some(super::win::Edit { default, max: 29, ..Default::default() }),
            buttons: vec!["Redeem".into(), "Cancel".into()],
            default: 0,
            cancel: Some(1),
            ..Default::default()
        };
        let o = super::win::show(d, Some(c.window.clone()));
        if c.is_set() || o.button != Some(0) {
            return Payload::status(E_ABORT);
        }
        let code = o.text.trim().to_ascii_uppercase().replace('-', "");
        let codes: Value = std::fs::read_to_string(store_dir().join("codes.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        match codes[&code].as_str() {
            Some(id) if allowed.is_empty() || allowed.iter().any(|a| a.eq_ignore_ascii_case(id)) => Payload::status(grant(id, 1)),
            _ => {
                super::win::show(Dialog { title: "Redeem a code".into(), text: "This code isn't valid.".into(), buttons: vec!["OK".into()], cancel: Some(0), ..Default::default() }, Some(c.window.clone()));
                Payload::status(E_NOT_FOUND)
            }
        }
    }))
}

unsafe extern "system" fn associated_ui_async(_t: This, _ctx: usize, id: *const u8, kinds: u32, block: *mut c_void) -> i32 {
    let base = cstr(id).unwrap_or_default();
    xasync::run_with(block, OP_UI, Box::new(move |c| {
        let items: Vec<Product> = catalog().into_iter().filter(|p| p.kind & kinds != 0 && !p.store_id.eq_ignore_ascii_case(&base)).collect();
        let text = if items.is_empty() { "There are no add-ons for this game.".to_string() } else { items.iter().map(|p| format!("{}  {}{}", p.title, p.formatted, if owns(&p.store_id) { "  (owned)" } else { "" })).collect::<Vec<_>>().join("\n") };
        super::win::show(Dialog { title: "Add-ons".into(), text, buttons: vec!["Close".into()], cancel: Some(0), ..Default::default() }, Some(c.window.clone()));
        Payload::status(if c.is_set() { E_ABORT } else { S_OK })
    }))
}

unsafe extern "system" fn product_page_async(_t: This, _ctx: usize, id: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    xasync::run_with(block, OP_UI, Box::new(move |c| {
        let text = match find(&id) {
            Some(p) => format!("{}\n\n{}\n\n{}{}", p.title, p.description, p.formatted, if owns(&p.store_id) { "  (owned)" } else { "" }),
            None => format!("This item isn't available.\n\n{id}"),
        };
        super::win::show(Dialog { title: "Microsoft Store".into(), text, buttons: vec!["Close".into()], cancel: Some(0), ..Default::default() }, Some(c.window.clone()));
        Payload::status(if c.is_set() { E_ABORT } else { S_OK })
    }))
}

unsafe extern "system" fn rate_async(_t: This, _ctx: usize, block: *mut c_void) -> i32 {
    xasync::run_with(block, OP_RATE, Box::new(move |c| {
        let d = Dialog {
            title: format!("Rate {}", title().display),
            text: "How would you rate this game?".into(),
            buttons: vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into(), "Not now".into()],
            default: 4,
            cancel: Some(5),
            ..Default::default()
        };
        let o = super::win::show(d, Some(c.window.clone()));
        if c.is_set() {
            return Payload::status(E_ABORT);
        }
        let rated = matches!(o.button, Some(b) if b < 5);
        if let Some(b) = o.button.filter(|&b| b < 5) {
            let _g = state_lock().lock();
            let mut s = load_state();
            s["rating"] = json!(b + 1);
            save_state(&s);
        }
        Payload::value(rated)
    }))
}

unsafe extern "system" fn rate_result(_t: This, block: *mut c_void, out: *mut bool) -> i32 {
    xasync::value(block, OP_RATE, out)
}

unsafe extern "system" fn status_result(_t: This, block: *mut c_void) -> i32 {
    xasync::status(block)
}

unsafe extern "system" fn updates_async(_t: This, _ctx: usize, block: *mut c_void) -> i32 {
    xasync::run(block, OP_UPDATES, Payload::data(Vec::new()))
}

unsafe extern "system" fn updates_count(_t: This, block: *mut c_void, count: *mut u32) -> i32 {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr >= 0 {
        *count = (n / 34) as u32;
    }
    hr
}

unsafe extern "system" fn updates_result(_t: This, block: *mut c_void, count: u32, out: *mut u8) -> i32 {
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr < 0 {
        return hr;
    }
    if (count as usize) * 34 < n {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    xasync::bytes(block, OP_UPDATES, n, out as *mut c_void, core::ptr::null_mut())
}

unsafe extern "system" fn download_updates_async(_t: This, _ctx: usize, ids: *const *const u8, n: usize, block: *mut c_void) -> i32 {
    let want = str_list(ids, n);
    let hr = if want.iter().all(|w| *w == super::package::package_id()) { S_OK } else { E_NOT_FOUND };
    xasync::run(block, OP_UI, Payload::status(hr))
}

unsafe extern "system" fn install_packages_async(_t: This, _ctx: usize, ids: *const *const u8, n: usize, block: *mut c_void) -> i32 {
    let want = str_list(ids, n);
    let mut b = Vec::new();
    for w in &want {
        if !owns(w) {
            return xasync::run(block, OP_PACKAGES, Payload::status(E_NOT_FOUND));
        }
        let pid = if w.eq_ignore_ascii_case(&game_store_id()) { super::package::package_id() } else { w.clone() };
        let mut e = [0u8; 33];
        let s = pid.as_bytes();
        e[..s.len().min(32)].copy_from_slice(&s[..s.len().min(32)]);
        b.extend_from_slice(&e);
    }
    xasync::run(block, OP_PACKAGES, Payload::data(b))
}

unsafe extern "system" fn install_packages_count(_t: This, block: *mut c_void, count: *mut u32) -> i32 {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr >= 0 {
        *count = (n / 33) as u32;
    }
    hr
}

unsafe extern "system" fn install_packages_result(_t: This, block: *mut c_void, count: u32, out: *mut u8) -> i32 {
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr < 0 {
        return hr;
    }
    if (count as usize) * 33 < n {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    xasync::bytes(block, OP_PACKAGES, n, out as *mut c_void, core::ptr::null_mut())
}

unsafe extern "system" fn package_identifier(_t: This, id: *const u8, size: usize, buf: *mut u8) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    let pid = if id.eq_ignore_ascii_case(&game_store_id()) { super::package::package_id() } else if owns(&id) { id } else { return E_NOT_FOUND };
    if buf.is_null() || size < pid.len() + 1 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(pid.as_ptr(), buf, pid.len());
    *buf.add(pid.len()) = 0;
    S_OK
}

struct License {
    magic: u64,
    id: String,
}

const LMAGIC: u64 = 0x4C49_4345_4E53_4521;

unsafe extern "system" fn acquire_package_license_async(_t: This, _ctx: usize, package: *const u8, block: *mut c_void) -> i32 {
    let pkg = cstr(package).unwrap_or_default();
    xasync::run(block, OP_LICENSE, Payload::value(Box::into_raw(Box::new(License { magic: LMAGIC, id: pkg })) as usize))
}

unsafe extern "system" fn acquire_durables_license_async(_t: This, _ctx: usize, id: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    xasync::run(block, OP_LICENSE, Payload::value(Box::into_raw(Box::new(License { magic: LMAGIC, id })) as usize))
}

unsafe extern "system" fn acquire_license_result(_t: This, block: *mut c_void, out: *mut usize) -> i32 {
    xasync::value(block, OP_LICENSE, out)
}

unsafe extern "system" fn is_license_valid(_t: This, license: usize) -> bool {
    match (license as *const License).as_ref() {
        Some(l) if l.magic == LMAGIC => l.id == super::package::package_id() || owns(&l.id),
        _ => false,
    }
}

unsafe extern "system" fn close_license(_t: This, license: usize) {
    if let Some(l) = (license as *mut License).as_mut() {
        if l.magic == LMAGIC {
            l.magic = 0;
            drop(Box::from_raw(l as *mut License));
        }
    }
}

pub const CAN_ACQUIRE_SIZE: usize = 12;

fn can_payload(sku: &str, status: u32) -> Payload {
    let mut b = vec![0u8; CAN_ACQUIRE_SIZE];
    let s = sku.as_bytes();
    b[..s.len().min(4)].copy_from_slice(&s[..s.len().min(4)]);
    b[8..12].copy_from_slice(&status.to_le_bytes());
    Payload::data(b)
}

unsafe extern "system" fn can_store_id_async(_t: This, _ctx: usize, id: *const u8, block: *mut c_void) -> i32 {
    let Some(id) = cstr(id) else { return E_INVALIDARG };
    let p = match find(&id) {
        Some(p) if owns(&p.store_id) => can_payload("0010", 1),
        Some(_) => can_payload("0010", 0),
        None => can_payload("", 2),
    };
    xasync::run(block, OP_CAN, p)
}

unsafe extern "system" fn can_package_async(_t: This, _ctx: usize, package: *const u8, block: *mut c_void) -> i32 {
    let pkg = cstr(package).unwrap_or_default();
    let p = if pkg == super::package::package_id() { can_payload("0010", 1) } else if owns(&pkg) { can_payload("0010", 1) } else { can_payload("", 0) };
    xasync::run(block, OP_CAN, p)
}

unsafe extern "system" fn can_result(_t: This, block: *mut c_void, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp = [0u8; CAN_ACQUIRE_SIZE];
    let hr = xasync::bytes(block, OP_CAN, CAN_ACQUIRE_SIZE, tmp.as_mut_ptr() as *mut c_void, core::ptr::null_mut());
    if hr >= 0 {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), out, CAN_ACQUIRE_SIZE);
    }
    hr
}

pub fn game_license_bytes() -> [u8; 104] {
    let mut b = [0u8; 104];
    let sku = format!("{}/0010", game_store_id());
    let s = sku.as_bytes();
    b[..s.len().min(17)].copy_from_slice(&s[..s.len().min(17)]);
    b[18] = 1;
    b
}

unsafe extern "system" fn game_license_async(_t: This, _ctx: usize, block: *mut c_void) -> i32 {
    xasync::run(block, OP_GAME_LICENSE, Payload::data(game_license_bytes().to_vec()))
}

unsafe extern "system" fn game_license_result(_t: This, block: *mut c_void, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp = [0u8; 104];
    let hr = xasync::bytes(block, OP_GAME_LICENSE, 104, tmp.as_mut_ptr() as *mut c_void, core::ptr::null_mut());
    if hr >= 0 {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), out, 104);
    }
    hr
}

unsafe extern "system" fn addons_async(_t: This, _ctx: usize, block: *mut c_void) -> i32 {
    let mut b = Vec::new();
    for p in catalog().into_iter().filter(|p| p.kind & 0x2 != 0 && owns(&p.store_id)) {
        let mut e = [0u8; 96];
        let sku = format!("{}/0010", p.store_id);
        let s = sku.as_bytes();
        e[..s.len().min(17)].copy_from_slice(&s[..s.len().min(17)]);
        e[82] = 1;
        b.extend_from_slice(&e);
    }
    xasync::run(block, OP_ADDONS, Payload::data(b))
}

unsafe extern "system" fn addons_count(_t: This, block: *mut c_void, count: *mut u32) -> i32 {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr >= 0 {
        *count = (n / 96) as u32;
    }
    hr
}

unsafe extern "system" fn addons_result(_t: This, block: *mut c_void, count: u32, out: *mut u8) -> i32 {
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr < 0 {
        return hr;
    }
    if (count as usize) * 96 < n {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    xasync::bytes(block, OP_ADDONS, n, out as *mut c_void, core::ptr::null_mut())
}

pub fn store_token(kind: &str, extra: Value) -> String {
    let u = super::user::user();
    let body = json!({
        "typ": kind,
        "iss": "urn:xboxemulator:store",
        "xuid": u.xuid.to_string(),
        "titleId": title().title_id.map(|t| format!("{t:08X}")),
        "productId": game_store_id(),
        "iat": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "ext": extra,
    });
    let b64 = |s: &str| {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let d = s.as_bytes();
        let mut o = String::new();
        for c in d.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..c.len() + 1 {
                o.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        o
    };
    format!("{}.{}.", b64(r#"{"alg":"none","typ":"JWT"}"#), b64(&body.to_string()))
}

fn string_payload(s: String) -> Payload {
    let mut b = s.into_bytes();
    b.push(0);
    Payload::data(b)
}

unsafe extern "system" fn collections_id_async(_t: This, _ctx: usize, ticket: *const u8, publisher_user: *const u8, block: *mut c_void) -> i32 {
    let t = store_token("collections", json!({"serviceTicket": cstr(ticket).unwrap_or_default().len(), "publisherUserId": cstr(publisher_user).unwrap_or_default()}));
    xasync::run(block, OP_STRING, string_payload(t))
}

unsafe extern "system" fn purchase_id_async(_t: This, _ctx: usize, ticket: *const u8, publisher_user: *const u8, block: *mut c_void) -> i32 {
    let t = store_token("purchase", json!({"serviceTicket": cstr(ticket).unwrap_or_default().len(), "publisherUserId": cstr(publisher_user).unwrap_or_default()}));
    xasync::run(block, OP_STRING, string_payload(t))
}

unsafe extern "system" fn license_token_async(_t: This, _ctx: usize, ids: *const *const u8, n: usize, custom: *const u8, block: *mut c_void) -> i32 {
    let products: Vec<String> = str_list(ids, n).into_iter().filter(|p| owns(p)).collect();
    let t = store_token("license", json!({"products": products, "custom": cstr(custom).unwrap_or_default()}));
    xasync::run(block, OP_STRING, string_payload(t))
}

unsafe extern "system" fn string_size(_t: This, block: *mut c_void, size: *mut usize) -> i32 {
    xasync::size(block, size)
}

unsafe extern "system" fn string_result(_t: This, block: *mut c_void, size: usize, buf: *mut u8) -> i32 {
    xasync::bytes(block, OP_STRING, size, buf as *mut c_void, core::ptr::null_mut())
}

static LICENSE_CHANGED: Registry = Registry::new();
static LICENSE_LOST: Registry = Registry::new();

unsafe extern "system" fn register_changed(_t: This, owner: usize, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    LICENSE_CHANGED.register(owner, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_changed(_t: This, owner: usize, token: u64, wait: bool) -> bool {
    LICENSE_CHANGED.unregister(Some(owner), token, wait)
}

unsafe extern "system" fn register_lost(_t: This, license: usize, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    if !is_license_valid(core::ptr::null(), license) {
        return E_INVALIDARG;
    }
    LICENSE_LOST.register(license, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_lost(_t: This, license: usize, token: u64, wait: bool) -> bool {
    LICENSE_LOST.unregister(Some(license), token, wait)
}

unsafe extern "system" fn is_availability_purchasable(_t: This, a: *const u8) -> bool {
    if a.is_null() {
        return false;
    }
    let id = core::ptr::read_unaligned(a as *const *const u8);
    let end = core::ptr::read_unaligned(a.add(96) as *const i64);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    !id.is_null() && (end == 0 || end > now)
}

fn slots() -> Vec<(usize, usize)> {
    vec![
        (3, create_context as usize),
        (4, close_handle as usize),
        (5, associated_async as usize),
        (6, products_result as usize),
        (7, products_by_id_async as usize),
        (8, products_result as usize),
        (9, entitled_async as usize),
        (10, products_result as usize),
        (11, current_game_async as usize),
        (12, products_result as usize),
        (13, for_package_async as usize),
        (14, products_result as usize),
        (15, enumerate as usize),
        (16, has_more_pages as usize),
        (17, next_page_async as usize),
        (18, products_result as usize),
        (19, close_query as usize),
        (20, acquire_package_license_async as usize),
        (21, acquire_license_result as usize),
        (22, is_license_valid as usize),
        (23, close_license as usize),
        (24, can_store_id_async as usize),
        (25, can_result as usize),
        (26, can_package_async as usize),
        (27, can_result as usize),
        (28, game_license_async as usize),
        (29, game_license_result as usize),
        (30, addons_async as usize),
        (31, addons_count as usize),
        (32, addons_result as usize),
        (33, consumable_async as usize),
        (34, consumable_result as usize),
        (35, fulfill_async as usize),
        (36, fulfill_result as usize),
        (37, collections_id_async as usize),
        (38, string_size as usize),
        (39, string_result as usize),
        (40, purchase_id_async as usize),
        (41, string_size as usize),
        (42, string_result as usize),
        (43, license_token_async as usize),
        (44, string_size as usize),
        (45, string_result as usize),
        (49, purchase_ui_async as usize),
        (50, status_result as usize),
        (51, rate_async as usize),
        (52, rate_result as usize),
        (53, redeem_ui_async as usize),
        (54, status_result as usize),
        (55, updates_async as usize),
        (56, updates_count as usize),
        (57, updates_result as usize),
        (58, download_updates_async as usize),
        (59, status_result as usize),
        (60, download_updates_async as usize),
        (61, status_result as usize),
        (62, install_packages_async as usize),
        (63, install_packages_count as usize),
        (64, install_packages_result as usize),
        (65, package_identifier as usize),
        (66, register_changed as usize),
        (67, unregister_changed as usize),
        (68, register_lost as usize),
        (69, unregister_lost as usize),
        (73, associated_ui_async as usize),
        (74, status_result as usize),
        (75, product_page_async as usize),
        (76, status_result as usize),
    ]
}

pub fn interfaces() -> Vec<Iface> {
    let mut v2 = slots();
    v2.extend([(71, acquire_durables_license_async as usize), (72, acquire_license_result as usize)]);
    let mut v3 = v2.clone();
    v3.push((70, is_availability_purchasable as usize));
    vec![
        Iface { clsid: CLSID, iid: IID, name: "XStore(de3dbdd4)", slots: slots() },
        Iface { clsid: CLSID, iid: IID_V2, name: "XStore(2d42fea5)", slots: v2 },
        Iface { clsid: CLSID, iid: IID_V3, name: "XStore(60b09f4e)", slots: v3 },
    ]
}
