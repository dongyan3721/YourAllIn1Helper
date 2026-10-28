//! Native ACL coverage: mocked page tests cannot detect a missing event capability.
use std::{collections::HashSet, fs, path::Path};

#[test]
fn sticky_windows_can_register_and_release_lifecycle_event_listeners() {
    let label = mod_sticky_note::StickyNoteModule::window_label_for("qa-note");
    let capabilities = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut permissions = HashSet::new();
    for entry in fs::read_dir(capabilities).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let applies = value["windows"].as_array().is_some_and(|windows| {
            windows.iter().filter_map(|v| v.as_str()).any(|pattern| {
                pattern == label
                    || pattern.strip_suffix('*').is_some_and(|prefix| label.starts_with(prefix))
            })
        });
        if applies {
            for permission in value["permissions"].as_array().unwrap() {
                if let Some(id) = permission.as_str() {
                    permissions.insert(id.to_owned());
                }
            }
        }
    }
    for permission in ["core:event:allow-listen", "core:event:allow-unlisten"] {
        assert!(permissions.contains(permission), "{label} lacks {permission}; shutdown acknowledgements cannot work");
    }
}
