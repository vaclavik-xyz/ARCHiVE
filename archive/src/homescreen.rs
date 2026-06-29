//! Reconstruct the Home Screen layout from SpringBoard's `IconState.plist`.
//!
//! The plist is a normal (unencrypted-content) backup file, so this works on any
//! backup. Its shape is a dictionary with a `buttonBar` array (the dock) and an
//! `iconLists` array of home-screen pages; each page is an array of icon entries.
//! An entry is either a *leaf* (an app or web clip, identified by a bundle/display
//! identifier) or a *folder* (a dict with `listType == "folder"` carrying its own
//! `iconLists` of pages). iOS does not nest folders, so folders expand exactly one
//! level deep. Every layer is read leniently — an unrecognized entry is skipped,
//! never panics.

use std::io::Cursor;

use plist::Value;
use serde::Serialize;

/// Backup domain holding SpringBoard state.
pub const DOMAIN: &str = "HomeDomain";

/// Relative path of the home-screen layout plist.
pub const PATH: &str = "Library/SpringBoard/IconState.plist";

/// One placed icon on the home screen, dock, or inside a folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IconSlot {
    /// Where the icon sits: `dock`, `page N` (1-based), or `folder:<name>`.
    pub container: String,
    /// 0-based position within its container (continuous across a folder's pages).
    pub position: u32,
    /// `app`, `webclip`, or `folder`.
    pub kind: String,
    /// Bundle/display identifier (apps & web clips); empty for an unnamed folder.
    pub identifier: String,
    /// Display name — the folder's title, or an app/web-clip caption when present;
    /// empty otherwise.
    pub label: String,
}

/// Classification of a raw icon entry.
enum Entry {
    Leaf { kind: String, identifier: String, label: String },
    Folder { name: String, identifier: String, pages: Vec<Vec<Value>> },
}

/// Parse the home-screen layout from `IconState.plist` bytes. Total: returns an
/// empty list on any parse failure rather than erroring.
pub fn parse(bytes: &[u8]) -> Vec<IconSlot> {
    let Ok(v) = Value::from_reader(Cursor::new(bytes)) else {
        return Vec::new();
    };
    let Some(root) = v.as_dictionary() else {
        return Vec::new();
    };
    let mut out = Vec::new();

    if let Some(dock) = root.get("buttonBar").and_then(Value::as_array) {
        emit_page(dock, "dock", &mut out);
    }
    if let Some(pages) = root.get("iconLists").and_then(Value::as_array) {
        for (i, page) in pages.iter().enumerate() {
            if let Some(items) = page.as_array() {
                emit_page(items, &format!("page {}", i + 1), &mut out);
            }
        }
    }
    out
}

/// Emit one page (or the dock). Folders push their own slot first, then their
/// contents flattened across the folder's pages with continuous positions.
fn emit_page(items: &[Value], container: &str, out: &mut Vec<IconSlot>) {
    for (pos, item) in items.iter().enumerate() {
        match classify(item) {
            Some(Entry::Leaf { kind, identifier, label }) => {
                out.push(IconSlot { container: container.to_string(), position: pos as u32, kind, identifier, label });
            }
            Some(Entry::Folder { name, identifier, pages }) => {
                out.push(IconSlot {
                    container: container.to_string(),
                    position: pos as u32,
                    kind: "folder".to_string(),
                    identifier,
                    label: name.clone(),
                });
                let fcontainer = format!("folder:{name}");
                let mut fpos = 0u32;
                for page in &pages {
                    for entry in page {
                        // Folders never nest, so only leaves are expected here.
                        if let Some(Entry::Leaf { kind, identifier, label }) = classify(entry) {
                            out.push(IconSlot {
                                container: fcontainer.clone(),
                                position: fpos,
                                kind,
                                identifier,
                                label,
                            });
                            fpos += 1;
                        }
                    }
                }
            }
            None => {}
        }
    }
}

/// Classify a raw entry as a leaf (app / web clip / widget / widget stack) or a
/// folder. A bare string entry (the common case — apps are stored as plain bundle
/// id strings) is treated as an app. Returns `None` for entries with no usable
/// identifier or name.
fn classify(item: &Value) -> Option<Entry> {
    if let Some(s) = item.as_string() {
        return (!s.is_empty()).then(|| Entry::Leaf {
            kind: "app".to_string(),
            identifier: s.to_string(),
            label: String::new(),
        });
    }
    let d = item.as_dictionary()?;

    // Folder: a "folder" listType (its pages live under `iconLists`).
    let is_folder = d.get("listType").and_then(Value::as_string) == Some("folder")
        || d.contains_key("iconLists");
    if is_folder {
        let name = str_key(d, &["displayName", "title"]).unwrap_or_default();
        let identifier = str_key(d, &["displayIdentifier", "bundleIdentifier"]).unwrap_or_default();
        let pages = d
            .get("iconLists")
            .and_then(Value::as_array)
            .map(|ps| ps.iter().filter_map(|p| p.as_array().cloned()).collect())
            .unwrap_or_default();
        return Some(Entry::Folder { name, identifier, pages });
    }

    // Widget stack (iOS 14+): rotates through several widgets held in `elements`.
    // Its own `displayIdentifier` is an opaque UUID, so surface the member apps
    // instead, joined into the label.
    if let Some(elements) = d.get("elements").and_then(Value::as_array) {
        let apps = dedup(elements.iter().filter_map(|e| {
            e.as_dictionary()
                .and_then(|w| str_key(w, &["containerBundleIdentifier", "widgetIdentifier", "bundleIdentifier"]))
        }));
        return Some(Entry::Leaf {
            kind: "widget-stack".to_string(),
            identifier: String::new(),
            label: apps.join(", "),
        });
    }

    // Standalone widget.
    if d.get("elementType").and_then(Value::as_string) == Some("widget")
        || d.contains_key("widgetIdentifier")
    {
        let identifier =
            str_key(d, &["containerBundleIdentifier", "widgetIdentifier", "bundleIdentifier"]).unwrap_or_default();
        let label = str_key(d, &["bundleIdentifier"]).unwrap_or_default();
        return Some(Entry::Leaf { kind: "widget".to_string(), identifier, label });
    }

    // App or web clip.
    let identifier = str_key(d, &["displayIdentifier", "bundleIdentifier", "bundleID"]).unwrap_or_default();
    let label = str_key(d, &["displayName", "title"]).unwrap_or_default();
    if identifier.is_empty() && label.is_empty() {
        return None;
    }
    let is_webclip = identifier.contains("webapp")
        || identifier.contains("webclip")
        || d.contains_key("webClipURL")
        || d.contains_key("webClip");
    let kind = if is_webclip { "webclip" } else { "app" };
    Some(Entry::Leaf { kind: kind.to_string(), identifier, label })
}

/// Collect strings preserving first-seen order, dropping duplicates.
fn dedup(it: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for s in it {
        if !seen.contains(&s) {
            seen.push(s);
        }
    }
    seen
}

fn str_key(d: &plist::Dictionary, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(s) = d.get(k).and_then(Value::as_string).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::{Dictionary, Value};

    fn to_bytes(v: &Value) -> Vec<u8> {
        let mut buf = Vec::new();
        v.to_writer_xml(&mut buf).unwrap();
        buf
    }

    fn app(id: &str) -> Value {
        let mut d = Dictionary::new();
        d.insert("displayIdentifier".into(), Value::String(id.into()));
        Value::Dictionary(d)
    }

    #[test]
    fn parses_dock_pages_and_folder() {
        // Folder "Social" with two apps, sitting on page 1 after one app.
        let mut folder = Dictionary::new();
        folder.insert("listType".into(), Value::String("folder".into()));
        folder.insert("displayName".into(), Value::String("Social".into()));
        folder.insert(
            "iconLists".into(),
            Value::Array(vec![Value::Array(vec![app("com.x.fb"), app("com.x.ig")])]),
        );

        let mut top = Dictionary::new();
        top.insert("buttonBar".into(), Value::Array(vec![app("com.apple.mobilephone")]));
        top.insert(
            "iconLists".into(),
            Value::Array(vec![
                Value::Array(vec![app("com.apple.mobilesafari"), Value::Dictionary(folder)]),
                Value::Array(vec![app("com.apple.Maps")]),
            ]),
        );

        let slots = parse(&to_bytes(&Value::Dictionary(top)));
        // dock(1) + page1: safari + folder + 2 folder apps + page2: maps = 6
        assert_eq!(slots.len(), 6);

        assert_eq!(slots[0].container, "dock");
        assert_eq!(slots[0].identifier, "com.apple.mobilephone");

        assert_eq!(slots[1].container, "page 1");
        assert_eq!(slots[1].position, 0);
        assert_eq!(slots[1].identifier, "com.apple.mobilesafari");

        let folder_slot = &slots[2];
        assert_eq!(folder_slot.kind, "folder");
        assert_eq!(folder_slot.label, "Social");
        assert_eq!(folder_slot.position, 1);

        assert_eq!(slots[3].container, "folder:Social");
        assert_eq!(slots[3].position, 0);
        assert_eq!(slots[3].identifier, "com.x.fb");
        assert_eq!(slots[4].container, "folder:Social");
        assert_eq!(slots[4].position, 1);
        assert_eq!(slots[4].identifier, "com.x.ig");

        assert_eq!(slots[5].container, "page 2");
        assert_eq!(slots[5].identifier, "com.apple.Maps");
    }

    #[test]
    fn detects_webclip_and_bare_string_entries() {
        let mut clip = Dictionary::new();
        clip.insert("displayIdentifier".into(), Value::String("com.apple.webapp.ABC".into()));
        clip.insert("displayName".into(), Value::String("Web App".into()));

        let mut top = Dictionary::new();
        top.insert(
            "iconLists".into(),
            // a bare-string app id (older iOS) + a web clip
            Value::Array(vec![Value::Array(vec![
                Value::String("com.legacy.app".into()),
                Value::Dictionary(clip),
            ])]),
        );

        let slots = parse(&to_bytes(&Value::Dictionary(top)));
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].kind, "app");
        assert_eq!(slots[0].identifier, "com.legacy.app");
        assert_eq!(slots[1].kind, "webclip");
        assert_eq!(slots[1].label, "Web App");
    }

    #[test]
    fn widget_stack_surfaces_member_apps() {
        // A widget stack: opaque UUID displayIdentifier, members under `elements`.
        let mut w1 = Dictionary::new();
        w1.insert("elementType".into(), Value::String("widget".into()));
        w1.insert("containerBundleIdentifier".into(), Value::String("com.apple.weather".into()));
        let mut w2 = Dictionary::new();
        w2.insert("elementType".into(), Value::String("widget".into()));
        w2.insert("containerBundleIdentifier".into(), Value::String("com.apple.Maps".into()));

        let mut stack = Dictionary::new();
        stack.insert("displayIdentifier".into(), Value::String("7DAB7ADC-UUID".into()));
        stack.insert("iconType".into(), Value::String("widgetStack".into()));
        stack.insert("elements".into(), Value::Array(vec![Value::Dictionary(w1), Value::Dictionary(w2)]));

        let mut top = Dictionary::new();
        top.insert("iconLists".into(), Value::Array(vec![Value::Array(vec![Value::Dictionary(stack)])]));

        let slots = parse(&to_bytes(&Value::Dictionary(top)));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].kind, "widget-stack");
        assert_eq!(slots[0].identifier, ""); // UUID hidden
        assert_eq!(slots[0].label, "com.apple.weather, com.apple.Maps");
    }

    #[test]
    fn standalone_widget_uses_container_app() {
        let mut w = Dictionary::new();
        w.insert("widgetIdentifier".into(), Value::String("com.apple.weather".into()));
        w.insert("bundleIdentifier".into(), Value::String("com.apple.weather.widget".into()));
        let mut top = Dictionary::new();
        top.insert("iconLists".into(), Value::Array(vec![Value::Array(vec![Value::Dictionary(w)])]));

        let slots = parse(&to_bytes(&Value::Dictionary(top)));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].kind, "widget");
        assert_eq!(slots[0].identifier, "com.apple.weather");
    }

    #[test]
    fn malformed_or_empty_never_panics() {
        assert!(parse(b"").is_empty());
        assert!(parse(b"not a plist").is_empty());
        assert!(parse(&to_bytes(&Value::Array(vec![]))).is_empty());
    }
}
