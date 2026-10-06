//! Where the collection's impulse responses are installed, and where Browse opens.
//!
//! **One folder for the whole collection, per user and outside the plugin** (owner, 2026-09-16):
//! the platform's local data directory, then `mxm/impulses` — `%LOCALAPPDATA%\mxm\impulses` on
//! Windows, `~/Library/Application Support/mxm/impulses` on macOS and `~/.local/share/mxm/impulses`
//! on Linux. It holds `mxm-room-ir`'s release in that crate's own layout, and it does not depend on
//! the plugin format: a VST3 build reads the same folder. The local data directory, not the roaming
//! config directory user presets live under, because the catalogue is hundreds of megabytes.
//!
//! **The root is injected**, as the preset library's is: the editor resolves it once and hands it to
//! Browse, so a test never reads the folder of whoever runs it.
//!
//! **What is in the folder is the factory set** (the owner, 2026-09-28: *every impulse file should
//! be a reverb preset… what is in that folder is the default presets*). [`scan`] finds every WAV in
//! it when the editor opens, names each after its file and files it under the first folder it sits
//! in; nothing names a file in code. A found preset carries only where its file is, and [`load`]
//! reads it when it is chosen, on the preset path's background worker — so a hundred rooms cost a
//! directory walk to list, and a project keeps the response it loaded, not the path.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use mxm_preset::{Category, Found, Preset};

use crate::params::MxmFxConvolutionParams;
use crate::response::ResponseState;

/// The key a found preset's `state` names its file under, relative to the folder, with `/`
/// between folders on every platform.
pub const IMPULSE_KEY: &str = "impulse";

/// How deep the scan looks below the folder, the most rooms it takes, and the most entries of any
/// kind it reads. The catalogue is three levels deep — family, room, file — with a hundred rooms
/// in some five hundred entries; the bounds are there so a folder somebody filled with something
/// else cannot stall the editor opening, and the entry budget counts everything read, not only
/// the WAVs.
const MAX_DEPTH: usize = 4;
const MAX_FILES: usize = 2_000;
const MAX_ENTRIES: usize = 20_000;

/// The impulses folder under `data_local`, the platform's local data directory.
pub fn root_under(data_local: Option<PathBuf>) -> Option<PathBuf> {
    data_local.map(|dir| dir.join("mxm").join("impulses"))
}

/// This user's impulses folder; `None` where the platform has no local data directory.
pub fn root() -> Option<PathBuf> {
    root_under(dirs::data_local_dir())
}

/// The folder Browse opens in: the impulses folder when it is installed, and otherwise nothing, so
/// the dialog opens wherever the platform chooses. It never creates the folder. It reads the file
/// system, so it runs on the dialog's thread, never inside an editor frame.
pub fn browse_start(root: Option<&Path>) -> Option<PathBuf> {
    root.filter(|dir| dir.is_dir()).map(Path::to_path_buf)
}

/// **Every WAV in the folder, as a factory preset**, in folder then name order.
///
/// Each is Init with that room: the name is the file's up to its first dot, in words
/// (`concrete-stairwell.far.stereo.wav` is *Concrete stairwell*), and the group is the first
/// folder under the root (`halls/…` is *Halls*); a file in the root itself has none. A name found
/// twice is numbered, because a preset's name and origin are its key. A missing folder, and any
/// folder that cannot be read, is simply fewer presets.
pub fn scan(root: Option<&Path>, params: &MxmFxConvolutionParams) -> Vec<Found> {
    let Some(root) = root.filter(|dir| dir.is_dir()) else {
        return Vec::new();
    };
    let mut walk_state = Walk {
        files: Vec::new(),
        entries_left: MAX_ENTRIES,
    };
    walk(root, root, 0, &mut walk_state);
    let files = walk_state.files;
    let mut rooms: Vec<(Option<String>, String, String)> = files
        .into_iter()
        .filter_map(|relative| {
            let mut parts: Vec<&str> = relative.split('/').collect();
            let file = parts.pop()?;
            let name = words(file.split('.').next().unwrap_or(file));
            let group = (!parts.is_empty()).then(|| words(parts[0]));
            (!name.is_empty()).then(|| (group, name, relative.clone()))
        })
        .collect();
    rooms.sort_by(|a, b| {
        let key = |room: &(Option<String>, String, String)| {
            (
                room.0.as_deref().map(str::to_lowercase),
                room.1.to_lowercase(),
            )
        };
        key(a).cmp(&key(b))
    });
    // **Every name given out once**, compared without case, because a preset's name and origin are
    // its key. Init is the list's first factory preset, so it is taken before any room; a repeat is
    // numbered with the first number free, checked against every name so far — a numbered name can
    // be a real one too (`init.wav` and `init-2.wav`).
    let mut taken: HashSet<String> = HashSet::from([mxm_preset::INIT_NAME.to_lowercase()]);
    rooms
        .into_iter()
        .map(|(group, name, relative)| {
            let mut unique = name.clone();
            let mut number = 1;
            while !taken.insert(unique.to_lowercase()) {
                number += 1;
                unique = format!("{name} {number}");
            }
            let mut preset = Preset::init(params);
            preset.name = unique;
            preset.category = Category::Fx;
            preset.state = Some(serde_json::json!({ IMPULSE_KEY: relative }));
            Found { preset, group }
        })
        .collect()
}

/// What a scan has found, and how many more entries it may read.
struct Walk {
    files: Vec<String>,
    entries_left: usize,
}

/// The WAVs under `dir`, as paths relative to `root` with `/` between folders, in name order.
/// Reads no more than the entries left — counted before they are sorted, so a folder of a million
/// files costs the budget and no more.
fn walk(root: &Path, dir: &Path, depth: usize, walk_state: &mut Walk) {
    if depth > MAX_DEPTH || walk_state.files.len() >= MAX_FILES || walk_state.entries_left == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Counted as attempted, errors included, so a folder that fails to read entry after entry
    // still spends the budget.
    let attempted: Vec<_> = entries.take(walk_state.entries_left).collect();
    walk_state.entries_left -= attempted.len();
    let mut entries: Vec<_> = attempted.into_iter().filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if walk_state.files.len() >= MAX_FILES {
            return;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(root, &path, depth + 1, walk_state);
        } else if kind.is_file()
            && is_wav(&path)
            && let Ok(relative) = path.strip_prefix(root)
        {
            let parts: Option<Vec<&str>> = relative
                .components()
                .map(|component| component.as_os_str().to_str())
                .collect();
            if let Some(parts) = parts {
                walk_state.files.push(parts.join("/"));
            }
        }
    }
}

fn is_wav(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("wave")
        })
}

/// A file or folder name in words: `-` and `_` as spaces, and a capital to start.
fn words(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The file a found preset's `state` names, if that is what it is — rather than a response
/// embedded whole, which is what a saved preset carries.
pub fn reference(state: &serde_json::Value) -> Option<&str> {
    let object = state.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get(IMPULSE_KEY)?.as_str()
}

/// Reads a found preset's file into a response, as Browse does. The path must stay inside the
/// folder — plain names between `/`, no `..` and no root — so a preset cannot point anywhere else.
pub fn load(root: Option<&Path>, relative: &str) -> Result<ResponseState, String> {
    let root = root.ok_or_else(|| "no impulses folder is installed".to_owned())?;
    let inside = !relative.is_empty()
        && Path::new(relative)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !inside {
        return Err(format!("{relative} is not in the impulses folder"));
    }
    let path = relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part));
    crate::response::decode_wav(&path).map_err(|why| format!("{relative}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The folder is scanned into one preset per WAV** (the owner, 2026-09-28), laid out as the
    /// catalogue is — family, room, file, with the room's metadata beside it — plus a file in the
    /// root, a second room of the same name, and what is not a room.
    #[test]
    fn the_folder_is_scanned_into_one_preset_per_wav() {
        let root = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = |relative: &str| {
            let path = relative
                .split('/')
                .fold(root.clone(), |path, part| path.join(part));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        };
        file("halls/cinema/cinema.far.stereo.wav");
        file("halls/cinema/metadata/cinema.far.stereo.json");
        file("rooms/walk-in-closet/walk-in-closet.far.stereo.wav");
        file("rooms/Cinema.WAV");
        file("loose_room.wave");
        file("init.wav");
        file("init-2.wav");
        file("notes.txt");
        file(".hidden/secret.wav");

        let params = MxmFxConvolutionParams::default();
        let found = scan(Some(&root), &params);
        let listed: Vec<(&str, Option<&str>, &str)> = found
            .iter()
            .map(|found| {
                let state = found
                    .preset
                    .state
                    .as_ref()
                    .expect("a found preset names its file");
                (
                    found.preset.name.as_str(),
                    found.group.as_deref(),
                    reference(state).expect("a reference"),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("Init 2", None, "init.wav"),
                ("Init 2 2", None, "init-2.wav"),
                ("Loose room", None, "loose_room.wave"),
                (
                    "Cinema",
                    Some("Halls"),
                    "halls/cinema/cinema.far.stereo.wav"
                ),
                ("Cinema 2", Some("Rooms"), "rooms/Cinema.WAV"),
                (
                    "Walk in closet",
                    Some("Rooms"),
                    "rooms/walk-in-closet/walk-in-closet.far.stereo.wav"
                ),
            ]
        );
        // Init's settings with that room, and filed as an effect.
        let init = Preset::init(&params);
        for found in &found {
            assert_eq!(found.preset.params, init.params);
            assert_eq!(found.preset.category, Category::Fx);
        }

        assert!(scan(None, &params).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(scan(Some(&root), &params).is_empty(), "no folder, no rooms");
    }

    #[test]
    fn the_folder_is_mxm_impulses_under_the_local_data_directory() {
        let base = PathBuf::from("data-local");
        assert_eq!(
            root_under(Some(base.clone())),
            Some(base.join("mxm").join("impulses"))
        );
        assert_eq!(root_under(None), None);
    }

    #[test]
    fn browse_opens_in_the_folder_only_once_it_is_installed() {
        let base = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-impulses-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = root_under(Some(base.clone())).unwrap();

        assert_eq!(browse_start(None), None);
        assert_eq!(browse_start(Some(&root)), None, "not installed");
        assert!(!root.exists(), "looking must not create the folder");

        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        std::fs::write(&root, b"not a folder").unwrap();
        assert_eq!(browse_start(Some(&root)), None, "a file of that name");
        std::fs::remove_file(&root).unwrap();

        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(browse_start(Some(&root)), Some(root.clone()));

        std::fs::remove_dir_all(&base).unwrap();
    }
}
