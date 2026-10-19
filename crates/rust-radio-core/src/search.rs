//! 939 台目录索引：按 contentId 取规范条目。
//!
//! 搜索走云听服务端（见 `docs/SEARCH-EVAL.md`），本地不再做关键词匹配；
//! 这个索引只负责把服务端返回的条目对上本地目录，补台别 / 省份 / 在播节目。

use std::collections::HashMap;

use crate::models::Station;

#[derive(Debug, Clone, Default)]
pub struct StationIndex {
    entries: Vec<Station>,
    /// contentId → entries 下标
    by_id: HashMap<String, usize>,
}

impl StationIndex {
    pub fn build(stations: &[Station]) -> Self {
        let entries = stations.to_vec();
        let mut by_id = HashMap::with_capacity(entries.len());
        for (i, s) in entries.iter().enumerate() {
            by_id.insert(s.content_id.clone(), i);
        }
        Self { entries, by_id }
    }

    /// 按 contentId 取索引里的规范条目（补台别/省份/在播节目用）
    pub fn get(&self, content_id: &str) -> Option<&Station> {
        self.by_id.get(content_id).map(|&i| &self.entries[i])
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stations(&self) -> Vec<Station> {
        self.entries.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, title: &str, tag: &str) -> Station {
        Station {
            content_id: id.into(),
            title: title.into(),
            tag_name: tag.into(),
            ..Default::default()
        }
    }

    #[test]
    fn looks_up_by_content_id() {
        let idx =
            StationIndex::build(&[st("639", "中国之声", "中央"), st("640", "经济之声", "中央")]);
        assert_eq!(idx.get("639").map(|s| s.title.as_str()), Some("中国之声"));
        assert_eq!(idx.get("640").map(|s| s.tag_name.as_str()), Some("中央"));
        assert!(idx.get("nope").is_none());
        assert_eq!(idx.len(), 2);
    }
}
