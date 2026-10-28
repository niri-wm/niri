use std::collections::{HashSet, VecDeque};
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use smithay::input::pointer::CursorIcon;

use super::{read_bounded, SourceCandidate};

const MAX_INDEX_BYTES: usize = 64 * 1024;
const MAX_THEME_DEPTH: usize = 32;
const MAX_THEME_NODES: usize = 256;

/// Resolved directories only: decoded cursor sources remain local to a worker job.
pub(super) struct ThemeResolver {
    directories: Vec<(usize, PathBuf)>,
}

impl ThemeResolver {
    pub(super) fn load(name: &str, cancelled: &impl Fn() -> bool) -> Self {
        let roots = search_roots(
            env::var_os("HOME").as_deref(),
            env::var_os("XCURSOR_PATH").as_deref(),
            env::var_os("XDG_DATA_HOME").as_deref(),
            env::var_os("XDG_DATA_DIRS").as_deref(),
        );
        Self::with_roots(name, &roots, cancelled)
    }

    pub(super) fn with_roots(name: &str, roots: &[PathBuf], cancelled: &impl Fn() -> bool) -> Self {
        let mut directories = Vec::new();
        let mut pending = VecDeque::from([(PathBuf::from(name), 0)]);
        let mut names = HashSet::from([PathBuf::from(name)]);
        let mut seen = HashSet::new();
        // Breadth first makes depth authoritative even with diamonds and cycles.
        while let Some((name, depth)) = pending.pop_front() {
            if cancelled() || directories.len() == MAX_THEME_NODES {
                break;
            }
            let paths = name.is_absolute().then(|| name.clone()).into_iter().chain(
                roots
                    .iter()
                    .take(if name.is_absolute() {
                        0
                    } else {
                        MAX_THEME_NODES
                    })
                    .map(|root| root.join(&name)),
            );
            for path in paths {
                if cancelled() || directories.len() == MAX_THEME_NODES {
                    break;
                }
                if !path.is_dir() {
                    continue;
                }
                let Ok(identity) = path.canonicalize() else {
                    continue;
                };
                if !seen.insert(identity) {
                    continue;
                }
                if cancelled() {
                    break;
                }
                let mut parents = read_bounded(&path.join("index.theme"), MAX_INDEX_BYTES)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .map(|text| inherited_themes(&text))
                    .unwrap_or_default();
                directories.push((depth, path));
                if cancelled() || depth == MAX_THEME_DEPTH {
                    continue;
                }
                if parents.is_empty() && name != Path::new("default") {
                    parents.push(PathBuf::from("default"));
                }
                for parent in parents {
                    if cancelled() || names.len() == MAX_THEME_NODES {
                        break;
                    }
                    if names.insert(parent.clone()) {
                        pending.push_back((parent, depth + 1));
                    }
                }
            }
        }
        Self { directories }
    }

    pub(super) fn candidates(
        &self,
        icon: CursorIcon,
        bitmaps: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Vec<SourceCandidate> {
        let mut candidates = Vec::new();
        for (order, (depth, directory)) in self.directories.iter().enumerate() {
            if cancelled() {
                break;
            }
            // Do not even look up bitmap paths unless explicitly opted in.
            for scalable in [true, false].into_iter().take(if bitmaps { 2 } else { 1 }) {
                let subdir = if scalable {
                    "cursors_scalable"
                } else {
                    "cursors"
                };
                for (alias, name) in std::iter::once(icon.name())
                    .chain(icon.alt_names().iter().copied())
                    .enumerate()
                {
                    if cancelled() {
                        break;
                    }
                    let path = directory.join(subdir).join(name);
                    let matches = if scalable {
                        path.is_dir()
                    } else {
                        path.is_file()
                    };
                    if matches {
                        candidates.push((*depth, !scalable, alias, order, path));
                    }
                }
            }
        }
        // Selected-theme bitmaps still precede inherited SVGs. Within a depth,
        // format and alias precede the stable root/parent traversal order.
        candidates
            .sort_by_key(|(depth, bitmap, alias, order, _)| (*depth, *bitmap, *alias, *order));
        let mut seen = HashSet::new();
        candidates
            .into_iter()
            .filter_map(|(_, bitmap, _, _, path)| {
                if cancelled() {
                    return None;
                }
                let identity = path.canonicalize().ok()?;
                seen.insert(identity).then_some(SourceCandidate {
                    path,
                    scalable: !bitmap,
                    parsed: None,
                })
            })
            .collect()
    }
}

fn inherited_themes(text: &str) -> Vec<PathBuf> {
    let mut icon_theme = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            icon_theme = line == "[Icon Theme]";
        } else if icon_theme {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "Inherits" {
                    return value
                        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                        .filter(|name| !name.is_empty())
                        .take(MAX_THEME_NODES)
                        .map(PathBuf::from)
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

fn expand_home(path: PathBuf, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~") {
        Ok(suffix) => Some(home?.join(suffix)),
        Err(_) => Some(path),
    }
}

// Explicit inputs keep path policy independently testable without mutating the environment.
fn search_roots(
    home: Option<&OsStr>,
    xcursor_path: Option<&OsStr>,
    data_home: Option<&OsStr>,
    data_dirs: Option<&OsStr>,
) -> Vec<PathBuf> {
    fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
        value.filter(|value| !value.is_empty())
    }
    let home = nonempty(home).map(Path::new);
    let mut roots = if let Some(paths) = nonempty(xcursor_path) {
        env::split_paths(&paths)
            .filter(|path| !path.as_os_str().is_empty())
            .filter_map(|path| expand_home(path, home))
            .collect()
    } else {
        let mut roots = Vec::new();
        if let Some(path) = nonempty(data_home) {
            roots.extend(expand_home(path.into(), home).map(|path| path.join("icons")));
        } else if let Some(home) = home {
            roots.push(home.join(".local/share/icons"));
        }
        if let Some(home) = home {
            roots.push(home.join(".icons"));
        }
        if let Some(paths) = nonempty(data_dirs) {
            roots.extend(
                env::split_paths(&paths)
                    .filter(|path| !path.as_os_str().is_empty())
                    .filter_map(|path| expand_home(path, home))
                    .map(|path| path.join("icons")),
            );
        } else {
            roots.push("/usr/local/share/icons".into());
            roots.push("/usr/share/icons".into());
        }
        roots.push("/usr/share/pixmaps".into());
        if let Some(home) = home {
            roots.push(home.join(".cursors"));
        }
        roots.push("/usr/share/cursors/xorg-x11".into());
        roots
    };
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    roots.truncate(MAX_THEME_NODES);
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_paths_append_icons_and_preserve_root_order() {
        let roots = search_roots(
            Some(OsStr::new("/home/test")),
            None,
            Some(OsStr::new("/custom/data")),
            Some(OsStr::new("/first:/second::/first")),
        );
        assert_eq!(
            roots,
            [
                "/custom/data/icons",
                "/home/test/.icons",
                "/first/icons",
                "/second/icons",
                "/usr/share/pixmaps",
                "/home/test/.cursors",
                "/usr/share/cursors/xorg-x11",
            ]
            .map(PathBuf::from)
        );
        let defaults = search_roots(Some(OsStr::new("/home/test")), None, None, None);
        assert_eq!(defaults[0], Path::new("/home/test/.local/share/icons"));
        assert_eq!(defaults[1], Path::new("/home/test/.icons"));
        assert_eq!(defaults[2], Path::new("/usr/local/share/icons"));
        assert_eq!(defaults[3], Path::new("/usr/share/icons"));
        assert_eq!(
            search_roots(
                Some(OsStr::new("/home/test")),
                Some(OsStr::new("")),
                None,
                None
            ),
            defaults,
        );
    }

    #[test]
    fn xcursor_path_replaces_defaults_and_expands_only_home_components() {
        assert_eq!(
            search_roots(
                Some(OsStr::new("/home/test")),
                Some(OsStr::new("~/icons::~:~other/icons:/override:~/icons")),
                Some(OsStr::new("/ignored")),
                Some(OsStr::new("/also-ignored")),
            ),
            [
                "/home/test/icons",
                "/home/test/",
                "~other/icons",
                "/override"
            ]
            .map(PathBuf::from),
        );
        assert_eq!(
            search_roots(None, Some(OsStr::new("~/icons:~:/override")), None, None),
            [PathBuf::from("/override")],
        );
        let roots = search_roots(
            Some(OsStr::new("/home/test")),
            None,
            Some(OsStr::new("~/data")),
            Some(OsStr::new("~/shared")),
        );
        assert_eq!(roots[0], Path::new("/home/test/data/icons"));
        assert_eq!(roots[2], Path::new("/home/test/shared/icons"));
    }

    #[test]
    fn inheritance_parser_reads_all_ordered_parents_only_in_icon_theme() {
        assert_eq!(
            inherited_themes("[Other]\nInherits=wrong\n[Icon Theme]\n Inherits = first, second;third;;\n[Other]\nInherits=wrong\n"),
            ["first", "second", "third"].map(PathBuf::from),
        );
        assert!(inherited_themes("[Other]\nInherits=wrong\n").is_empty());
        let names = std::iter::repeat_n("parent", MAX_THEME_NODES + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            inherited_themes(&format!("[Icon Theme]\nInherits={names}")).len(),
            MAX_THEME_NODES
        );
    }

    #[test]
    fn traversal_bounds_depth_nodes_and_honors_cancellation() {
        let fixture = crate::cursor::tests::policy_theme();
        for index in 0..MAX_THEME_DEPTH + 2 {
            let path = fixture.join(format!("depth-{index}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("index.theme"),
                format!("[Icon Theme]\nInherits=depth-{}\n", index + 1),
            )
            .unwrap();
        }
        let roots = [fixture.clone()];
        let theme = ThemeResolver::with_roots("depth-0", &roots, &|| false);
        assert_eq!(theme.directories.len(), MAX_THEME_DEPTH + 1);
        assert_eq!(theme.directories.last().unwrap().0, MAX_THEME_DEPTH);
        let parents = (0..MAX_THEME_NODES + 1)
            .map(|index| format!("node-{index}"))
            .collect::<Vec<_>>();
        for name in &parents {
            let path = fixture.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("index.theme"), "[Icon Theme]\nInherits=wide\n").unwrap();
        }
        std::fs::create_dir(fixture.join("wide")).unwrap();
        std::fs::write(
            fixture.join("wide/index.theme"),
            format!("[Icon Theme]\nInherits={}\n", parents.join(",")),
        )
        .unwrap();
        let theme = ThemeResolver::with_roots("wide", &roots, &|| false);
        assert_eq!(theme.directories.len(), MAX_THEME_NODES);
        assert!(ThemeResolver::with_roots("wide", &roots, &|| true)
            .directories
            .is_empty());
        assert!(theme
            .candidates(CursorIcon::Default, true, &|| true)
            .is_empty());
        let checks = std::cell::Cell::new(0);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() >= 4
        };
        let interrupted = ThemeResolver::with_roots("wide", &roots, &cancelled);
        assert_eq!(interrupted.directories.len(), 1);
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
