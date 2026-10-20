// Saved window arrangements, after iTerm2's "Window Arrangements": every
// window's tabs, split tree and per-pane working directory, stored as
// `<config dir>/arrangements/<name>.toml` and restored into new windows.
//
// Splits are binary in Rio (`ContextGrid::split_panel` wraps the current
// panel and the new one in a fresh container), so a tab's layout maps to
// a binary tree that restores by replaying the same splits.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Arrangement {
    #[serde(default, rename = "window")]
    pub windows: Vec<WindowArrangement>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowArrangement {
    /// Outer position in physical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[i32; 2]>,
    /// Inner size in physical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    #[serde(default, rename = "active-tab")]
    pub active_tab: usize,
    #[serde(default, rename = "tab")]
    pub tabs: Vec<TabArrangement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabArrangement {
    /// The tab's rename, if any; computed titles are not saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The tab's color, as `#rrggbb` or `#rrggbbaa`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub layout: Layout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Layout {
    Pane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Split {
        direction: SplitDirection,
        /// Share of `first` along `direction`, clamped on restore.
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Right,
    Down,
}

impl Layout {
    /// Directory of the top-left pane: the one that exists before any
    /// split of this subtree is replayed.
    pub fn first_cwd(&self) -> Option<&str> {
        match self {
            Layout::Pane { cwd } => cwd.as_deref(),
            Layout::Split { first, .. } => first.first_cwd(),
        }
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn color_to_hex(color: [f32; 4]) -> String {
    let [r, g, b, a] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    if a == u8::MAX {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Parse `#rrggbb` / `#rrggbbaa` (the `#` is optional); `None` if malformed.
pub fn color_from_hex(hex: &str) -> Option<[f32; 4]> {
    let hex = hex.trim().trim_start_matches('#');
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return None;
    }
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map(|v| v as f32 / 255.0)
    };
    let alpha = if hex.len() == 8 { channel(6)? } else { 1.0 };
    Some([channel(0)?, channel(2)?, channel(4)?, alpha])
}

/// Restored ratios stay inside this range so no pane collapses.
pub fn clamp_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.05, 0.95)
    } else {
        0.5
    }
}

pub fn dir() -> PathBuf {
    rio_backend::config::config_dir_path().join("arrangements")
}

/// Turn a user-typed name into a file stem: path separators and control
/// characters become `_`, and a leading dot can't hide the file.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    stem.trim_start_matches('.').to_string()
}

fn path_for(name: &str) -> Option<PathBuf> {
    let stem = file_stem(name);
    (!stem.is_empty()).then(|| dir().join(format!("{stem}.toml")))
}

/// Saved arrangement names, sorted.
pub fn list() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "toml" {
                return None;
            }
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

pub fn save(name: &str, arrangement: &Arrangement) -> Result<(), String> {
    let path = path_for(name).ok_or("arrangement name is empty")?;
    let content = toml::to_string_pretty(arrangement).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(name: &str) -> Result<Arrangement, String> {
    let path = path_for(name).ok_or("arrangement name is empty")?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn delete(name: &str) -> Result<(), String> {
    let path = path_for(name).ok_or("arrangement name is empty")?;
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(cwd: &str) -> Box<Layout> {
        Box::new(Layout::Pane {
            cwd: Some(cwd.to_string()),
        })
    }

    #[test]
    fn toml_round_trip_keeps_the_split_tree() {
        let arrangement = Arrangement {
            windows: vec![WindowArrangement {
                position: Some([10, 20]),
                size: Some([1600, 900]),
                active_tab: 1,
                tabs: vec![
                    TabArrangement {
                        title: None,
                        color: None,
                        layout: Layout::Pane { cwd: None },
                    },
                    TabArrangement {
                        title: Some("api".to_string()),
                        color: Some("#ecb05f".to_string()),
                        layout: Layout::Split {
                            direction: SplitDirection::Right,
                            ratio: 0.6,
                            first: pane("/a"),
                            second: Box::new(Layout::Split {
                                direction: SplitDirection::Down,
                                ratio: 0.5,
                                first: pane("/b"),
                                second: pane("/c"),
                            }),
                        },
                    },
                ],
            }],
        };

        let text = toml::to_string_pretty(&arrangement).unwrap();
        let decoded: Arrangement = toml::from_str(&text).unwrap();
        assert_eq!(decoded, arrangement);
    }

    #[test]
    fn first_cwd_is_the_top_left_pane() {
        let layout = Layout::Split {
            direction: SplitDirection::Down,
            ratio: 0.5,
            first: Box::new(Layout::Split {
                direction: SplitDirection::Right,
                ratio: 0.5,
                first: pane("/top-left"),
                second: pane("/top-right"),
            }),
            second: pane("/bottom"),
        };
        assert_eq!(layout.first_cwd(), Some("/top-left"));
    }

    #[test]
    fn names_become_safe_file_stems() {
        assert_eq!(file_stem("  work  "), "work");
        assert_eq!(file_stem("a/b\\c:d"), "a_b_c_d");
        assert_eq!(file_stem("..hidden"), "hidden");
        assert!(path_for("   ").is_none());
    }

    #[test]
    fn tab_colors_round_trip_through_hex() {
        let orange = [236.0 / 255.0, 176.0 / 255.0, 95.0 / 255.0, 1.0];
        assert_eq!(color_to_hex(orange), "#ecb05f");
        assert_eq!(color_from_hex("#ecb05f"), Some(orange));

        let translucent = [1.0, 0.0, 0.0, 128.0 / 255.0];
        assert_eq!(color_to_hex(translucent), "#ff000080");
        assert_eq!(color_from_hex("ff000080"), Some(translucent));

        assert_eq!(color_from_hex("#ecb"), None);
        assert_eq!(color_from_hex("#zzzzzz"), None);
        assert_eq!(color_from_hex("#ecb05fé"), None);
    }

    #[test]
    fn ratio_is_clamped() {
        assert_eq!(clamp_ratio(0.0), 0.05);
        assert_eq!(clamp_ratio(1.0), 0.95);
        assert_eq!(clamp_ratio(f32::NAN), 0.5);
        assert_eq!(clamp_ratio(0.3), 0.3);
    }
}
