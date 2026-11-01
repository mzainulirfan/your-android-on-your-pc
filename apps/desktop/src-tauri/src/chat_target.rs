use crate::adb;
use crate::clipboard_image;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Finishing the chat app's own "send to" picker.
///
/// Android will not let an outside process put a picture onto the phone's clipboard (measured on
/// the test device: `service call clipboard` answers "Permission to access clipboard was denied"
/// and `cmd clipboard` has no shell implementation), so "paste the screenshot into the chat I am
/// already looking at" cannot be served through the clipboard at all. The hand-off in
/// [`crate::clipboard_image`] opens the app's picker; this module closes it by reading the
/// visible list (`uiautomator dump`) and tapping the remembered chat.
///
/// Two rules make that safe to run against a real conversation:
/// * a tap needs an **exact** label match and **exactly one** match in the dump, and
/// * a tap only happens on a screen this module opened itself (focused window + picker rows seen
///   before the first tap).
/// Anything else leaves the phone untouched and the panel asks for one click instead.

/// Saved destinations, one file per concern like credentials and wireless addresses.
const STORE_NAME: &str = "chat-targets.json";
const MAX_TOTAL: usize = 16;
const MAX_LABEL_CHARS: usize = 120;
/// A dump is ~55 KB on the test phone; anything much larger is a runaway or a corrupt pipe and
/// must not be parsed or shipped to the UI.
const MAX_DUMP_BYTES: usize = 2_000_000;
/// WhatsApp's contact row label. `uiautomator` marks these rows `clickable="false"` (a
/// RecyclerView handles item clicks itself, so the accessibility tree never says so) — that is
/// not a problem for `input tap`, which hit-tests by coordinates: the centre of the label is
/// inside the row that the user would have touched.
const ROW_ID_SUFFIX: &str = "contactpicker_row_name";
/// `com.whatsapp:id/send` — the green arrow on the caption/preview screen newer WhatsApp builds
/// show before a share actually leaves the phone.
const SEND_ID_SUFFIX: &str = "/id/send";
const SEND_LABELS: [&str; 2] = ["Kirim", "Send"];
/// One shell session on the phone: dump, print, remove. The file never survives the call, and no
/// caller data is part of the string (the serial stays its own validated argument).
const UI_DUMP: &str = "uiautomator dump /sdcard/yaop-ui.xml >/dev/null 2>&1; cat /sdcard/yaop-ui.xml; rm -f /sdcard/yaop-ui.xml";
/// Anything past this is not a real screen: displays above 20000 px do not exist, so a match like
/// that means the dump was misread.
const MAX_COORD: u32 = 20_000;
/// The two halves of the time limit need different sentences: before a tap nothing has been touched,
/// after the row tap the picture already sits inside a chat and the user has to look at the phone.
const SLOW_BEFORE_TAP: &str = "HP terlalu lambat menjawab, jadi YAOP berhenti sebelum mengetuk.";
const SLOW_AFTER_TAP: &str = "HP terlalu lambat menjawab — lihat layar HP, gambar mungkin masih antre.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bounds {
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
}

impl Bounds {
    fn is_sane(&self) -> bool {
        self.x1 < self.x2 && self.y1 < self.y2 && self.x2 <= MAX_COORD && self.y2 <= MAX_COORD
    }

    fn width(&self) -> u32 {
        self.x2 - self.x1
    }

    fn height(&self) -> u32 {
        self.y2 - self.y1
    }

    fn center(&self) -> (u32, u32) {
        ((self.x1 + self.x2) / 2, (self.y1 + self.y2) / 2)
    }
}

/// One element of the dumped hierarchy — only the attributes this module decides on.
#[derive(Clone, Debug)]
struct Node {
    text: String,
    description: String,
    resource_id: String,
    class: String,
    package: String,
    enabled: bool,
    bounds: Option<Bounds>,
}

impl Node {
    /// Label as the user reads it on the phone: the visible text, or the accessibility description
    /// when the app only fills that one.
    fn label(&self) -> String {
        let text = self.text.trim();
        if !text.is_empty() {
            return text.to_string();
        }
        self.description.trim().to_string()
    }

    /// A tappable box: enabled, inside the screen, and big enough not to be a 1 px artifact.
    fn tap_point(&self) -> Option<(u32, u32)> {
        let bounds = self.bounds?;
        if !self.enabled || bounds.width() < 8 || bounds.height() < 8 {
            return None;
        }
        Some(bounds.center())
    }
}

/// A chat the user can pick: label plus the point that would be tapped for it. Coordinates never
/// travel to the UI — a later click re-reads the phone and recomputes them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRow {
    pub label: String,
    pub x: u32,
    pub y: u32,
}

/// Why the picker was not finished, and what the panel should show instead.
#[derive(Clone, Debug)]
pub enum Finish {
    /// The image is in the chat (or the app's own send step was completed).
    Sent,
    /// The front window is not a picker this module knows — keep the old manual behaviour.
    NoRows,
    /// The remembered chat is not in the visible list.
    NotFound(Vec<ChatRow>),
    /// The label appears more than once, so no single row can be trusted.
    Ambiguous(Vec<ChatRow>),
    /// Something outside our control (screen off, another app in front, dump failing, budget out)
    /// — carries the note shown to the user.
    Aborted(String),
}

#[derive(Clone, Debug)]
pub enum RowMatch {
    One(ChatRow),
    /// Not on the screen at all. The caller already holds the visible list, so only the doubled
    /// case carries what it found.
    Missing,
    Duplicate(Vec<ChatRow>),
}

/// The list as the panel sees it: labels only.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerEvent {
    pub serial: String,
    pub package: String,
    /// `new` (no saved target yet), `not-found`, `ambiguous`, `stale` (the list moved under us).
    pub reason: &'static str,
    pub chats: Vec<String>,
}

impl PickerEvent {
    fn new(serial: &str, package: &str, reason: &'static str, rows: &[ChatRow]) -> Self {
        // Names can repeat on the phone (two contacts called "Ibu"); the panel keys its chips by
        // label, so a doubled name is offered once. Choosing it stays refused — see `find_row`.
        let mut chats: Vec<String> = Vec::with_capacity(rows.len());
        for row in rows {
            if !chats.contains(&row.label) {
                chats.push(row.label.clone());
            }
        }
        Self {
            serial: serial.to_string(),
            package: package.to_string(),
            reason,
            chats,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    id: String,
    serial: String,
    package: String,
    label: String,
}

/// What the settings page is allowed to see about a saved destination.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetView {
    pub id: String,
    pub serial: String,
    pub package: String,
    pub label: String,
}

/// The visible chats of a picker, in the order the phone draws them.
pub fn picker_rows(xml: &str, package: &str) -> Vec<ChatRow> {
    parse_nodes(xml)
        .iter()
        .filter(|node| node.package == package && node.resource_id.ends_with(ROW_ID_SUFFIX))
        .filter_map(|node| {
            let label = node.label();
            if label.is_empty() {
                return None;
            }
            let (x, y) = node.tap_point()?;
            Some(ChatRow { label, x, y })
        })
        .collect()
}

/// Exact-match only: no case folding, no substring, no accent tricks. Guessing a conversation from
/// a partial name is the one outcome this feature must never produce.
pub fn find_row(rows: &[ChatRow], label: &str) -> RowMatch {
    let matches: Vec<ChatRow> = rows
        .iter()
        .filter(|row| row.label == label)
        .cloned()
        .collect();
    match matches.len() {
        0 => RowMatch::Missing,
        1 => RowMatch::One(matches[0].clone()),
        _ => RowMatch::Duplicate(matches),
    }
}

/// The green arrow on the preview screen, if the phone is showing exactly one. Two nodes carrying
/// the same caption (icon + label) is a normal WhatsApp layout, so anything but a single candidate
/// is refused rather than guessed at.
pub fn send_button(xml: &str, package: &str) -> Option<ChatRow> {
    let found: Vec<ChatRow> = parse_nodes(xml)
        .iter()
        .filter(|node| node.package == package)
        .filter_map(|node| {
            let by_id = node.resource_id.ends_with(SEND_ID_SUFFIX);
            let by_label = node.class.contains("Button")
                && SEND_LABELS
                    .iter()
                    .any(|label| node.text == *label || node.description == *label);
            if !by_id && !by_label {
                return None;
            }
            let (x, y) = node.tap_point()?;
            Some(ChatRow {
                label: node.label(),
                x,
                y,
            })
        })
        .collect();
    if found.len() == 1 {
        found.into_iter().next()
    } else {
        None
    }
}

pub fn dump_ui(serial: &str) -> Result<String, String> {
    let text = adb::shell(serial, &[UI_DUMP])?;
    if text.len() > MAX_DUMP_BYTES {
        return Err("Daftar layar HP terlalu besar untuk dibaca.".to_string());
    }
    Ok(text)
}

/// The picker list for a chat app we just handed an image to, waiting briefly for it to come up.
/// Empty answer means "not a picker we know" and the caller keeps the manual behaviour.
pub fn visible_rows(serial: &str, package: &str) -> Vec<ChatRow> {
    match settle_on_picker(serial, package, &Deadline::new(serial)) {
        Settled::Rows(rows) => rows,
        _ => Vec::new(),
    }
}

enum Settled {
    /// Chats the phone is offering.
    Rows(Vec<ChatRow>),
    /// The front window is the right app but shows no chat list we recognise.
    NotPicker,
    /// Something else is in front, or the screen is off — carries the note for the user.
    Gone(String),
}

/// The whole picker interaction shares one wall-clock limit, because every step is an ADB round
/// trip that competes with the mirrored video for the same adbd connection.
struct Deadline {
    started: Instant,
    budget: Duration,
}

impl Deadline {
    fn new(serial: &str) -> Deadline {
        // A Wi-Fi round trip is a multiple of USB's, so the budget follows the transport. Individual
        // calls still have their own 20 s ADB timeout; this only stops us stacking several slow ones.
        Deadline {
            started: Instant::now(),
            budget: Duration::from_secs(if serial.contains(':') { 20 } else { 12 }),
        }
    }

    fn expired(&self) -> bool {
        self.started.elapsed() > self.budget
    }
}

/// `uiautomator dump` measured 3,4 s on the test device for a 34 KB screen, so reading the screen is
/// the expensive half of this feature: it happens only once the app is certainly in front, and at
/// most twice, since a picker that is still animating can answer with nodes but no chat rows.
fn settle_on_picker(serial: &str, package: &str, deadline: &Deadline) -> Settled {
    // This also guards the screen: a dark screen answers `mCurrentFocus=null`.
    let front = match focused_package(serial) {
        Some(front) => front,
        None => return Settled::Gone(lost(serial, "")),
    };
    if front != package {
        return Settled::Gone(lost(serial, &front));
    }

    for attempt in 0..2 {
        if deadline.expired() {
            return Settled::Gone(SLOW_BEFORE_TAP.to_string());
        }
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(400));
        }
        if let Ok(xml) = dump_ui(serial) {
            let rows = picker_rows(&xml, package);
            if !rows.is_empty() {
                return Settled::Rows(rows);
            }
        }
    }
    Settled::NotPicker
}

/// The image was handed to `package`, so a chat list of its own should be in front. Anything else
/// is explained rather than guessed at.
fn lost(serial: &str, seen: &str) -> String {
    if !seen.is_empty() {
        return format!("HP sudah berpindah ke {seen} sebelum daftar tujuan selesai.");
    }
    match focused_package(serial) {
        Some(front) => format!("HP sudah berpindah ke {front} sebelum daftar tujuan selesai."),
        None => "Layar HP mati atau tidak menampilkan aplikasi apa pun.".to_string(),
    }
}

/// Resolve the phone's own "send to" list for `label` and complete it. Only function in the app
/// that ever sends a tap.
pub fn finish_picker(serial: &str, package: &str, label: &str) -> Finish {
    let deadline = Deadline::new(serial);
    let rows = match settle_on_picker(serial, package, &deadline) {
        Settled::Rows(rows) => rows,
        Settled::NotPicker => return Finish::NoRows,
        Settled::Gone(note) => return Finish::Aborted(note),
    };

    let row = match find_row(&rows, label) {
        RowMatch::One(row) => row,
        RowMatch::Missing => return Finish::NotFound(rows),
        RowMatch::Duplicate(matches) => return Finish::Ambiguous(matches),
    };

    // The window the picker is in right now. If this string is still identical after the tap, the
    // tap did not take us anywhere and the image is still sitting unsent in the list.
    let before = clipboard_image::focus_window(serial);
    if tap(serial, row.x, row.y).is_err() {
        return Finish::Aborted("YAOP tidak bisa mengetuk daftar chat di HP.".to_string());
    }

    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(600));
        if deadline.expired() {
            // The row tap already happened, so say what stops here: nothing more is tapped.
            return Finish::Aborted(SLOW_AFTER_TAP.to_string());
        }
        let Ok(xml) = dump_ui(serial) else {
            continue;
        };
        if let Some(button) = send_button(&xml, package) {
            if tap(serial, button.x, button.y).is_err() {
                return Finish::Aborted("YAOP tidak bisa mengetuk tombol Kirim.".to_string());
            }
            return Finish::Sent;
        }
        if picker_rows(&xml, package).is_empty() {
            // No list left. Either the share went out directly (older builds) or the tap landed
            // elsewhere; the focused window says which.
            if clipboard_image::focus_window(serial) != before {
                return Finish::Sent;
            }
            return Finish::Aborted(
                "YAOP berhenti sebelum mengirim karena layar HP berubah.".to_string(),
            );
        }
    }

    Finish::Aborted(format!(
        "Chat \"{label}\" masih terlihat di daftar tujuan — YAOP tidak mengetuk lagi."
    ))
}

fn focused_package(serial: &str) -> Option<String> {
    clipboard_image::foreground_package(serial)
}

fn tap(serial: &str, x: u32, y: u32) -> Result<(), String> {
    // The coordinates are `u32` parsed out of validated bounds; the chat label never reaches a
    // command line at all.
    adb::shell(
        serial,
        &["input", "tap", &x.to_string(), &y.to_string()],
    )?;
    Ok(())
}

/// One-click path: the user picked a chat in the panel, so the phone is driven now and the choice
/// becomes the saved target for this device and app.
pub fn resolve_by_choice(
    app: &AppHandle,
    serial: &str,
    package: &str,
    label: &str,
) -> Result<String, String> {
    if !adb::is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }
    if !clipboard_image::is_safe_package(package) {
        return Err("Nama aplikasi tujuan tidak valid.".to_string());
    }
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err("Nama chat tujuan tidak bisa dipakai.".to_string());
    }

    match finish_picker(serial, package, label) {
        Finish::Sent => {
            remember(app, serial, package, label)?;
            Ok(format!("Terkirim ke {label}."))
        }
        Finish::NotFound(rows) | Finish::Ambiguous(rows) => {
            emit_choices(app, serial, package, "stale", &rows);
            Err("Daftar chat di HP sudah berubah — tekan Ctrl+V sekali lagi.".to_string())
        }
        Finish::NoRows => Err("HP sudah keluar dari daftar tujuan, jadi tidak ada yang diketuk."
            .to_string()),
        Finish::Aborted(note) => Err(note),
    }
}

pub fn emit_choices(
    app: &AppHandle,
    serial: &str,
    package: &str,
    reason: &'static str,
    rows: &[ChatRow],
) {
    if rows.is_empty() {
        return;
    }
    let event = PickerEvent::new(serial, package, reason, rows);
    let _ = app.emit("chat-picker", event);
}

/// The saved destination for one device and app, if there is one.
pub fn lookup(app: &AppHandle, serial: &str, package: &str) -> Option<String> {
    read(app)
        .into_iter()
        .find(|entry| entry.serial == serial && entry.package == package)
        .map(|entry| entry.label)
}

pub fn remember(
    app: &AppHandle,
    serial: &str,
    package: &str,
    label: &str,
) -> Result<TargetView, String> {
    if !adb::is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }
    if !clipboard_image::is_safe_package(package) {
        return Err("Nama aplikasi tujuan tidak valid.".to_string());
    }
    let label = label.trim();
    if label.is_empty() {
        return Err("Nama chat tujuan tidak boleh kosong.".to_string());
    }
    if label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!(
            "Nama chat tujuan terlalu panjang (maksimum {MAX_LABEL_CHARS} karakter)."
        ));
    }
    if label.chars().any(|character| character.is_control()) {
        return Err("Nama chat tujuan mengandung karakter yang tidak bisa dipakai.".to_string());
    }

    let mut entries = read(app);
    // One destination per device and app: choosing a new chat replaces the old one instead of
    // leaving a second entry that would decide which of the two a paste lands in.
    let existing = entries
        .iter()
        .position(|entry| entry.serial == serial && entry.package == package);
    if existing.is_none() && entries.len() >= MAX_TOTAL {
        return Err(format!(
            "Maksimal {MAX_TOTAL} tujuan chat tersimpan. Hapus sebagian di Setelan."
        ));
    }

    let entry = Stored {
        id: existing.map(|index| entries[index].id.clone()).unwrap_or_else(next_id),
        serial: serial.to_string(),
        package: package.to_string(),
        label: label.to_string(),
    };
    match existing {
        Some(index) => entries[index] = entry.clone(),
        None => entries.insert(0, entry.clone()),
    }
    write(app, &entries)?;
    Ok(entry.view())
}

/// Every saved destination across devices, for the settings page that groups them by serial.
pub fn all(app: &AppHandle) -> Vec<TargetView> {
    read(app).into_iter().map(|entry| entry.view()).collect()
}

pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut entries = read(app);
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == before {
        return Err("Tujuan chat sudah tidak tersimpan.".to_string());
    }
    write(app, &entries)
}

fn store_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join(STORE_NAME))
}

fn read(app: &AppHandle) -> Vec<Stored> {
    let Some(path) = store_path(app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // Unparseable stays unread rather than becoming an empty list: the next save must not wipe the
    // destinations the user already picked.
    serde_json::from_str::<Vec<Stored>>(&text).unwrap_or_default()
}

fn write(app: &AppHandle, entries: &[Stored]) -> Result<(), String> {
    let path = store_path(app).ok_or("Folder konfigurasi aplikasi tidak tersedia.")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Gagal membuat folder konfigurasi: {error}"))?;
    }
    let text = serde_json::to_string_pretty(entries)
        .map_err(|error| format!("Gagal menyusun daftar tujuan chat: {error}"))?;
    std::fs::write(&path, text).map_err(|error| format!("Gagal menyimpan tujuan chat: {error}"))
}

impl Stored {
    fn view(&self) -> TargetView {
        TargetView {
            id: self.id.clone(),
            serial: self.serial.clone(),
            package: self.package.clone(),
            label: self.label.clone(),
        }
    }
}

fn next_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default();
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed) as u64;
    format!("{millis:013x}{tick:03x}")
}

/// The nodes of a `uiautomator` dump, in document order. The dump is one long line of
/// attribute-only `<node …/>` tags; entity values escape `>` as `&gt;`, so the first raw `>` after
/// a tag really is its end. A truncated dump yields fewer nodes, never a panic — and a missing row
/// is the safe outcome, because no row means no tap.
fn parse_nodes(xml: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut rest = xml;
    while let Some(offset) = rest.find("<node ") {
        let tag = &rest[offset..];
        let Some(end) = tag.find('>') else {
            break;
        };
        nodes.push(Node::from_tag(&tag[..end]));
        rest = &tag[end + 1..];
    }
    nodes
}

impl Node {
    fn from_tag(tag: &str) -> Self {
        Self {
            text: decode_entities(attribute(tag, "text").unwrap_or("")),
            description: decode_entities(attribute(tag, "content-desc").unwrap_or("")),
            resource_id: attribute(tag, "resource-id").unwrap_or("").to_string(),
            class: attribute(tag, "class").unwrap_or("").to_string(),
            package: attribute(tag, "package").unwrap_or("").to_string(),
            enabled: attribute(tag, "enabled") != Some("false"),
            bounds: attribute(tag, "bounds").and_then(parse_bounds),
        }
    }
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Names and numbers, because a chat title can be an emoji and `uiautomator` writes those as
/// `&#127955;`. Exact matching against what the user sees depends on this.
fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let tail = &value[index..];
        let Some(ampersand) = tail.find('&') else {
            out.push_str(tail);
            break;
        };
        out.push_str(&tail[..ampersand]);
        index += ampersand;
        let rest = &value[index..];
        let Some(semi) = rest.find(';') else {
            out.push('&');
            index += 1;
            continue;
        };
        let name = &rest[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if name.starts_with('#') => {
                let digits = &name[1..];
                let number = match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse::<u32>().ok(),
                };
                number.and_then(char::from_u32)
            }
            _ => None,
        };
        match decoded {
            Some(character) => {
                out.push(character);
                index += semi + 1;
            }
            // Unknown or broken entity: keep the text as it is and move one byte forward, so a
            // stray `&` in a chat name survives into the comparison instead of eating the rest.
            None => {
                out.push('&');
                index += 1;
            }
        }
    }
    out
}

fn parse_bounds(value: &str) -> Option<Bounds> {
    let (left, right) = value.trim().split_once("][")?;
    let (x1, y1) = left.trim_start_matches('[').split_once(',')?;
    let (x2, y2) = right.trim_end_matches(']').split_once(',')?;
    let bounds = Bounds {
        x1: x1.trim().parse().ok()?,
        y1: y1.trim().parse().ok()?,
        x2: x2.trim().parse().ok()?,
        y2: y2.trim().parse().ok()?,
    };
    bounds.is_sane().then_some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed WhatsApp "Kirim ke…" dump with invented names — the shape matters, the people do
    /// not. Attribute order and quoting follow what `uiautomator` prints on the test device.
    const PICKER: &str = concat!(
        r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0">"#,
        r#"<node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.whatsapp" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" bounds="[0,0][1080,2340]" />"#,
        r#"<node index="1" text="Kirim ke…" resource-id="com.whatsapp:id/title" class="android.widget.TextView" package="com.whatsapp" content-desc="" clickable="false" enabled="true" bounds="[162,150][400,200]" />"#,
        r#"<node index="2" text="Sering dihubungi" resource-id="com.whatsapp:id/header_textview" class="android.widget.TextView" package="com.whatsapp" content-desc="" clickable="false" enabled="true" bounds="[40,400][400,450]" />"#,
        r#"<node index="3" text="Teh Nena" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Teh Nena" clickable="false" enabled="true" bounds="[162,817][301,861]" />"#,
        r#"<node index="4" text="Kak Aham" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Kak Aham" clickable="false" enabled="true" bounds="[162,1085][311,1129]" />"#,
        r#"<node index="5" text="&#127955;Kopi" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="&#127955;Kopi" clickable="false" enabled="true" bounds="[162,1200][320,1244]" />"#,
        r#"<node index="6" text="Kantor &amp; Pabrik" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="" clickable="false" enabled="true" bounds="[162,1300][420,1344]" />"#,
        r#"<node index="7" text="Diskon" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.android.shell" content-desc="Diskon" clickable="false" enabled="true" bounds="[162,1400][300,1444]" />"#,
        r#"<node index="8" text="" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="" clickable="false" enabled="true" bounds="[162,1500][300,1544]" />"#,
        r#"<node index="9" text="Ukir" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Ukir" clickable="false" enabled="false" bounds="[162,1600][300,1644]" />"#,
        r#"<node index="10" text="Rata" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Rata" clickable="false" enabled="true" bounds="[162,1700][166,1704]" />"#,
        r#"<node index="11" text="Ibu" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Ibu" clickable="false" enabled="true" bounds="[162,1800][300,1844]" />"#,
        r#"<node index="12" text="Ibu" resource-id="com.whatsapp:id/contactpicker_row_name" class="android.widget.TextView" package="com.whatsapp" content-desc="Ibu" clickable="false" enabled="true" bounds="[162,1900][300,1944]" />"#,
        r#"</hierarchy>"#
    );

    fn rows() -> Vec<ChatRow> {
        picker_rows(PICKER, "com.whatsapp")
    }

    #[test]
    fn rows_are_the_visible_chats_in_draw_order() {
        let visible = rows();
        let labels: Vec<&str> = visible.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Teh Nena", "Kak Aham", "🏓Kopi", "Kantor & Pabrik", "Ibu", "Ibu"]
        );
        // A tap lands on the middle of the label box, which is inside the row the user sees.
        assert_eq!(
            (visible[0].x, visible[0].y),
            ((162 + 301) / 2, (817 + 861) / 2)
        );
    }

    #[test]
    fn other_packages_and_unusable_boxes_are_never_offered() {
        let visible = rows();
        let labels: Vec<&str> = visible.iter().map(|row| row.label.as_str()).collect();
        assert!(!labels.contains(&"Diskon"), "row from another app leaked: {labels:?}");
        assert!(!labels.contains(&"Ukir"), "disabled row leaked: {labels:?}");
        assert!(!labels.contains(&"Rata"), "1x1 box leaked: {labels:?}");
        // Headers and empty labels are not chats either.
        assert!(!labels.contains(&"Sering dihubungi"));
        assert!(!labels.iter().any(|label| label.is_empty()));
    }

    #[test]
    fn matching_is_exact_and_refuses_duplicates() {
        assert!(matches!(
            find_row(&rows(), "Teh Nena"),
            RowMatch::One(ChatRow { x, y, .. }) if x == 231 && y == 839
        ));
        assert!(matches!(find_row(&rows(), "teh nena"), RowMatch::Missing));
        assert!(matches!(find_row(&rows(), "Nena"), RowMatch::Missing));
        assert!(matches!(find_row(&rows(), "Teh Nena "), RowMatch::Missing));
        let duplicate = find_row(&rows(), "Ibu");
        match duplicate {
            RowMatch::Duplicate(matches) => assert_eq!(matches.len(), 2),
            _ => panic!("a doubled name must be reported as ambiguous"),
        }
    }

    #[test]
    fn entities_decode_because_names_carry_emoji_and_ampersands() {
        assert_eq!(decode_entities("&#127955;Kopi"), "🏓Kopi");
        assert_eq!(decode_entities("Kantor &amp; Pabrik"), "Kantor & Pabrik");
        assert_eq!(decode_entities("&#x1F600;"), "😀");
        assert_eq!(decode_entities("&lt;&quot;&apos;&gt;"), "<\"'>");
        // A stray ampersand or an unknown name keeps its text instead of eating the rest.
        assert_eq!(decode_entities("A &amp; B & C"), "A & B & C");
        assert_eq!(decode_entities("5 & 6"), "5 & 6");
        assert_eq!(decode_entities("&#xFFFFFFFF"), "&#xFFFFFFFF");
    }

    #[test]
    fn malformed_bounds_are_rejected() {
        assert_eq!(parse_bounds("[162,817][301,861]"), Some(Bounds { x1: 162, y1: 817, x2: 301, y2: 861 }));
        assert_eq!(parse_bounds("[0,0][1080,2340]"), Some(Bounds { x1: 0, y1: 0, x2: 1080, y2: 2340 }));
        assert_eq!(parse_bounds(""), None);
        assert_eq!(parse_bounds("x"), None);
        assert_eq!(parse_bounds("[1,2][3]"), None);
        assert_eq!(parse_bounds("[9,9][9,9]"), None, "a box with no area is not tappable");
        assert_eq!(parse_bounds("[10,10][9,9]"), None, "inverted bounds are not tappable");
        assert_eq!(parse_bounds("[1,1][30000,30000]"), None, "past any real display");
        assert_eq!(parse_bounds("[1,1][abc,9]"), None);
        assert_eq!(parse_bounds("[1,1][999999999999,9]"), None);
    }

    #[test]
    fn a_truncated_dump_is_read_as_far_as_it_goes() {
        let cut = PICKER.split_once("content-desc=\"Kak Aham\"").unwrap().0;
        let visible = picker_rows(cut, "com.whatsapp");
        let labels: Vec<&str> = visible.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["Teh Nena"]);
        assert!(parse_nodes("<node index=\"0\" text=\"").is_empty());
    }

    #[test]
    fn the_send_button_needs_exactly_one_candidate() {
        let preview = concat!(
            r#"<hierarchy>"#,
            r#"<node text="Tulis keterangan" resource-id="com.whatsapp:id/caption" class="android.widget.EditText" package="com.whatsapp" content-desc="" enabled="true" bounds="[40,2100][900,2200]" />"#,
            r#"<node text="" resource-id="com.whatsapp:id/send" class="android.widget.ImageButton" package="com.whatsapp" content-desc="Send" enabled="true" bounds="[950,2110][1040,2200]" />"#,
            r#"</hierarchy>"#
        );
        let button = send_button(preview, "com.whatsapp").expect("send button found");
        assert_eq!(button.x, 995);

        let header_only = r#"<hierarchy><node text="Kirim ke…" resource-id="com.whatsapp:id/title" class="android.widget.TextView" package="com.whatsapp" enabled="true" bounds="[162,150][400,200]" /></hierarchy>"#;
        assert!(send_button(header_only, "com.whatsapp").is_none());

        let doubled = r#"<hierarchy><node text="Kirim" resource-id="com.whatsapp:id/send_label" class="android.widget.Button" package="com.whatsapp" enabled="true" bounds="[900,2100][1000,2200]" /><node text="Kirim" resource-id="com.whatsapp:id/other" class="android.widget.Button" package="com.whatsapp" enabled="true" bounds="[100,2100][200,2200]" /></hierarchy>"#;
        assert!(send_button(doubled, "com.whatsapp").is_none());
    }
}
