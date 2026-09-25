//! KINAVIS collision watch as a Signal K server WASM plugin.
//!
//! The server hands every delta to [`delta_handler`]; the watch reads own
//! vessel and the targets from them. Every few seconds [`poll`] assesses the
//! picture and sends back a `navigation.closestApproach` delta for each
//! closing target and a notification whenever a target's level changes. The
//! assessment itself is `kinavis-signalk`; this crate is the host interface
//! only, and the only place with `unsafe`.

use std::cell::RefCell;
use std::time::{SystemTime, UNIX_EPOCH};

use kinavis_signalk::{Emission, Instant, Level, Utc, Watch, WatchConfig};

const PLUGIN_ID: &str = "signalk-kinavis";
const PLUGIN_NAME: &str = "KINAVIS collision watch";
const PLUGIN_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "ownMmsi": {
      "type": "string",
      "title": "Own vessel's MMSI; leave empty to recognise own vessel by its GNSS receiver",
      "default": ""
    },
    "cpaLimitNm": {
      "type": "number",
      "title": "CPA limit (nautical miles): a closer approach is dangerous",
      "default": 1.0
    },
    "tcpaLimitMin": {
      "type": "number",
      "title": "TCPA limit (minutes): a dangerous approach sooner than this is an alarm, later a warning",
      "default": 20
    },
    "warnWithinMin": {
      "type": "number",
      "title": "Warning horizon (minutes): a dangerous approach further off than this is not reported",
      "default": 60
    },
    "staleAfterS": {
      "type": "number",
      "title": "Target report too old to assess after (seconds)",
      "default": 180
    },
    "restrictedVisibility": {
      "type": "boolean",
      "title": "Restricted visibility (COLREGs Rule 19 instead of Rules 11 to 18)",
      "default": false
    },
    "assessEveryS": {
      "type": "integer",
      "title": "Assess every (seconds)",
      "default": 2
    }
  }
}"#;

// =============================================================================
// Host interface
// =============================================================================

#[link(wasm_import_module = "env")]
extern "C" {
    fn sk_debug(ptr: *const u8, len: usize);
    fn sk_set_status(ptr: *const u8, len: usize);
    fn sk_set_error(ptr: *const u8, len: usize);
    fn sk_handle_message(ptr: *const u8, len: usize);
    fn sk_publish_notification(
        path_ptr: *const u8,
        path_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    ) -> i32;
}

fn debug(message: &str) {
    // SAFETY: the host reads `len` bytes from `ptr`, a live `&str`.
    unsafe { sk_debug(message.as_ptr(), message.len()) }
}

fn set_status(message: &str) {
    // SAFETY: as `debug`.
    unsafe { sk_set_status(message.as_ptr(), message.len()) }
}

fn set_error(message: &str) {
    // SAFETY: as `debug`.
    unsafe { sk_set_error(message.as_ptr(), message.len()) }
}

fn handle_message(json: &str) {
    // SAFETY: as `debug`.
    unsafe { sk_handle_message(json.as_ptr(), json.len()) }
}

fn publish_notification(path: &str, value: &str) -> i32 {
    // SAFETY: the host reads both strings, live for the call.
    unsafe { sk_publish_notification(path.as_ptr(), path.len(), value.as_ptr(), value.len()) }
}

/// Copies `text` into the host's buffer; bytes written.
fn write_out(text: &str, out: *mut u8, max_len: usize) -> i32 {
    let len = text.len().min(max_len);
    // SAFETY: the host passes a buffer of `max_len` writable bytes; at most
    // `len <= max_len` are written, from a source that does not overlap it.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), out, len) };
    i32::try_from(len).unwrap_or(i32::MAX)
}

/// Text the host placed in plugin memory.
///
/// # Safety
///
/// `ptr` and `len` describe bytes the host wrote into a buffer from
/// [`allocate`], live for the call.
unsafe fn read_in(ptr: *const u8, len: usize) -> String {
    // SAFETY: guaranteed by the caller.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Buffer for the host to write into.
#[no_mangle]
pub extern "C" fn allocate(size: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(size);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Frees a buffer from [`allocate`].
///
/// # Safety
///
/// `ptr` and `size` are exactly those of one [`allocate`] call, freed once.
#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: *mut u8, size: usize) {
    // SAFETY: guaranteed by the caller; length 0 drops no elements.
    drop(unsafe { Vec::from_raw_parts(ptr, 0, size) });
}

// =============================================================================
// Plugin
// =============================================================================

struct Plugin {
    watch: Watch,
    assess_every: u32,
    polls: u32,
    unreadable: u64,
}

thread_local! {
    static PLUGIN: RefCell<Option<Plugin>> = const { RefCell::new(None) };
}

/// Plugin id.
#[no_mangle]
pub extern "C" fn plugin_id(out: *mut u8, max_len: usize) -> i32 {
    write_out(PLUGIN_ID, out, max_len)
}

/// Plugin name.
#[no_mangle]
pub extern "C" fn plugin_name(out: *mut u8, max_len: usize) -> i32 {
    write_out(PLUGIN_NAME, out, max_len)
}

/// Configuration schema.
#[no_mangle]
pub extern "C" fn plugin_schema(out: *mut u8, max_len: usize) -> i32 {
    write_out(PLUGIN_SCHEMA, out, max_len)
}

/// Starts the watch with the configuration.
///
/// # Safety
///
/// `config_ptr` and `config_len` describe the configuration the host wrote.
#[no_mangle]
pub unsafe extern "C" fn plugin_start(config_ptr: *const u8, config_len: usize) -> i32 {
    // SAFETY: guaranteed by the caller.
    let text = unsafe { read_in(config_ptr, config_len) };
    let settings: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let config: WatchConfig = match serde_json::from_value(settings.clone()) {
        Ok(config) => config,
        Err(error) => {
            set_error(&format!("configuration unreadable, defaults used: {error}"));
            WatchConfig::default()
        }
    };
    let assess_every = settings
        .get("assessEveryS")
        .and_then(serde_json::Value::as_u64)
        .and_then(|seconds| u32::try_from(seconds).ok())
        .unwrap_or(2)
        .max(1);

    let mut watch = Watch::new(config);
    let own_mmsi = settings
        .get("ownMmsi")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|mmsi| !mmsi.is_empty());
    if let Some(mmsi) = own_mmsi {
        watch
            .picture_mut()
            .set_own_context(&format!("vessels.urn:mrn:imo:mmsi:{mmsi}"));
    }
    PLUGIN.with(|plugin| {
        *plugin.borrow_mut() = Some(Plugin {
            watch,
            assess_every,
            polls: 0,
            unreadable: 0,
        });
    });
    set_status("Watching");
    0
}

/// The present, by the host's clock.
fn now() -> Instant<Utc> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    Instant::from_unix_seconds(seconds)
}

/// Stops the watch.
#[no_mangle]
pub extern "C" fn plugin_stop() -> i32 {
    PLUGIN.with(|plugin| *plugin.borrow_mut() = None);
    set_status("Stopped");
    0
}

/// Reads one delta.
///
/// # Safety
///
/// `ptr` and `len` describe the delta the host wrote.
#[no_mangle]
pub unsafe extern "C" fn delta_handler(ptr: *const u8, len: usize) {
    // SAFETY: guaranteed by the caller.
    let text = unsafe { read_in(ptr, len) };
    PLUGIN.with(|plugin| {
        if let Some(plugin) = plugin.borrow_mut().as_mut() {
            if plugin.watch.ingest(&text).is_err() {
                plugin.unreadable = plugin.unreadable.saturating_add(1);
            }
        }
    });
}

/// Called every second: assesses every `assessEveryS` seconds.
#[no_mangle]
pub extern "C" fn poll() -> i32 {
    let emissions = PLUGIN.with(|plugin| {
        let mut plugin = plugin.borrow_mut();
        let plugin = plugin.as_mut()?;
        plugin.polls = plugin.polls.wrapping_add(1);
        if plugin.polls % plugin.assess_every != 0 {
            return None;
        }
        if !plugin.watch.picture().own_is_named() {
            set_status("Waiting for own vessel's position from its GNSS receiver");
            return None;
        }
        let (reports, emissions) = plugin.watch.assess(now());
        let alarms = reports
            .iter()
            .filter(|report| report.level == Level::Alarm)
            .count();
        let warnings = reports
            .iter()
            .filter(|report| report.level == Level::Warn)
            .count();
        set_status(&format!(
            "{} targets assessed: {alarms} alarm, {warnings} warning; {} deltas unreadable; own vessel {}",
            reports.len(),
            plugin.unreadable,
            plugin.watch.picture().own_context()
        ));
        Some(emissions)
    });
    for emission in emissions.unwrap_or_default() {
        match emission {
            Emission::Delta(delta) => {
                if let Ok(json) = serde_json::to_string(&delta) {
                    handle_message(&json);
                }
            }
            Emission::Notification { path, value } => {
                let status = publish_notification(&path, &value.to_string());
                if status != 0 {
                    debug(&format!("[WARN] notification refused: {path}"));
                }
            }
            _ => {}
        }
    }
    0
}
