//! 本地存储：收藏 / 最近播放 / 回听进度 / 设置，全部 JSON 落盘（原子替换）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use rust_radio_core::Station;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FavEntry {
    pub id: String,
    pub title: String,
    pub tag: String,
    pub image: String,
    pub subtitle: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecentEntry {
    pub id: String,
    pub title: String,
    pub tag: String,
    pub image: String,
    /// live | replay
    pub kind: String,
    pub program_name: String,
    pub played_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub favorites: Vec<FavEntry>,
    pub recents: Vec<RecentEntry>,
    /// program_id -> 秒
    pub progress: HashMap<String, f64>,
    pub volume: f32,
    pub quality_high: bool,
    /// auto | main | alt
    pub line: String,
    pub last_station: Option<String>,
    pub panel_open: bool,
    /// 侧栏「按地区」是否展开
    #[serde(default = "yes")]
    pub prov_open: bool,
    pub close_to_tray: bool,
    pub sleep_last_minutes: u32,
}

fn yes() -> bool {
    true
}

impl Default for Store {
    fn default() -> Self {
        Self {
            favorites: Vec::new(),
            recents: Vec::new(),
            progress: HashMap::new(),
            volume: 0.8,
            quality_high: true,
            line: "auto".into(),
            last_station: None,
            panel_open: true,
            prov_open: true,
            close_to_tray: false,
            sleep_last_minutes: 30,
        }
    }
}

impl Store {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("store.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join("store.json");
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn is_fav(&self, id: &str) -> bool {
        self.favorites.iter().any(|f| f.id == id)
    }

    pub fn toggle_fav(&mut self, s: &Station) -> bool {
        if let Some(pos) = self.favorites.iter().position(|f| f.id == s.content_id) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.insert(
                0,
                FavEntry {
                    id: s.content_id.clone(),
                    title: s.title.clone(),
                    tag: s.tag_name.clone(),
                    image: s.image.clone(),
                    subtitle: s.subtitle.clone(),
                    added_at: now(),
                },
            );
            true
        }
    }

    pub fn push_recent(&mut self, s: &Station, kind: &str, program: &str) {
        self.recents.retain(|r| r.id != s.content_id);
        self.recents.insert(
            0,
            RecentEntry {
                id: s.content_id.clone(),
                title: s.title.clone(),
                tag: s.tag_name.clone(),
                image: s.image.clone(),
                kind: kind.into(),
                program_name: program.into(),
                played_at: now(),
            },
        );
        self.recents.truncate(30);
    }

    pub fn recent_stations(&self) -> Vec<Station> {
        self.recents
            .iter()
            .map(|r| Station {
                content_id: r.id.clone(),
                title: r.title.clone(),
                tag_name: r.tag.clone(),
                subtitle: r.program_name.clone(),
                image: r.image.clone(),
                province_code: 0,
            })
            .collect()
    }

    pub fn fav_stations(&self) -> Vec<Station> {
        self.favorites
            .iter()
            .map(|r| Station {
                content_id: r.id.clone(),
                title: r.title.clone(),
                tag_name: r.tag.clone(),
                subtitle: r.subtitle.clone(),
                image: r.image.clone(),
                province_code: 0,
            })
            .collect()
    }
}

pub fn now() -> i64 {
    chrono::Local::now().timestamp()
}

/// 数据目录：Windows `%LOCALAPPDATA%\rust-radio`，macOS `~/Library/Application Support`，Linux `~/.local/share`
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUST_RADIO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let dir = default_data_dir();
    migrate_legacy(&dir);
    dir
}

fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(base).join("rust-radio");
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/rust-radio");
        }
    } else if let Some(base) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(base).join("rust-radio");
    } else if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/rust-radio");
    }
    std::env::temp_dir().join("rust-radio")
}

/// 老版本（yunting-radio）的数据目录：改名后第一次启动时把收藏/最近播放搬过来，
/// 避免升级即丢数据。
fn migrate_legacy(dir: &Path) {
    if dir.exists() {
        return;
    }
    let Some(parent) = dir.parent() else { return };
    let legacy = parent.join("yunting-radio");
    if !legacy.exists() {
        return;
    }
    if copy_dir(&legacy, dir).is_ok() {
        eprintln!(
            "已把旧数据目录 {} 迁移到 {}",
            legacy.display(),
            dir.display()
        );
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if std::fs::copy(entry.path(), &target).is_err() {
            // 单个文件失败（缓存被占用等）不应中断整体迁移
        }
    }
    Ok(())
}
