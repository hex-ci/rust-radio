//! 磁盘缓存：JSON 落盘 + TTL，原子替换写入。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let _ = std::fs::create_dir_all(&root);
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    fn age_of(path: &Path) -> Option<Duration> {
        let meta = std::fs::metadata(path).ok()?;
        meta.modified().ok()?.elapsed().ok()
    }

    /// 读缓存，超过 `ttl` 视为未命中。
    pub fn read<T: DeserializeOwned>(&self, key: &str, ttl: Duration) -> Option<T> {
        let path = self.path(key);
        let age = Self::age_of(&path)?;
        if age > ttl {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// 读缓存（不看时间），返回数据与年龄。
    pub fn read_stale<T: DeserializeOwned>(&self, key: &str) -> Option<(T, Duration)> {
        let path = self.path(key);
        let age = Self::age_of(&path)?;
        let text = std::fs::read_to_string(&path).ok()?;
        let value = serde_json::from_str(&text).ok()?;
        Some((value, age))
    }

    pub fn write<T: Serialize>(&self, key: &str, value: &T) -> std::io::Result<()> {
        let path = self.path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn touch(&self, key: &str) {
        let path = self.path(key);
        if let Ok(f) = std::fs::OpenOptions::new().append(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
    }

    pub fn remove(&self, key: &str) {
        let _ = std::fs::remove_file(self.path(key));
    }

    /// 缓存目录占用（字节）与文件数
    pub fn stats(&self) -> (u64, usize) {
        fn walk(dir: &Path, bytes: &mut u64, files: &mut usize) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, bytes, files);
                } else if let Ok(meta) = entry.metadata() {
                    *bytes += meta.len();
                    *files += 1;
                }
            }
        }
        let (mut b, mut f) = (0, 0);
        walk(&self.root, &mut b, &mut f);
        (b, f)
    }

    pub fn clear(&self) -> std::io::Result<()> {
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root)?;
        }
        std::fs::create_dir_all(&self.root)
    }

    /// 按子目录清理（如 `images`、`programs`）
    pub fn clear_sub(&self, sub: &str) -> std::io::Result<()> {
        let p = self.root.join(sub);
        if p.exists() {
            std::fs::remove_dir_all(&p)?;
        }
        std::fs::create_dir_all(&p)
    }
}
