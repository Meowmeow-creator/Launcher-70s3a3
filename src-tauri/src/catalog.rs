use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use super::xml::parse_file;

#[derive(Clone, Debug, Default)]
pub struct Quest {
    pub id: u32,
    pub name: String,
    pub grade: String,
    pub kind: String,
    pub min_level: i64,
    pub max_level: i64,
    pub supported: bool,
    pub reason: String,
    pub reward_grow: i64,
    pub reward_second_grow: i64,
    pub reward_expert: i64,
    pub condition: Vec<i64>,
    pub selection: Vec<i64>,
}

impl Quest {
    /// Hunt rows are [dungeon, difficulty, monster, goal] (width 4) or [.., enemy type, goal]
    /// (width 5) — the same stride the server's quest engine uses.
    pub fn hunt_stride(&self) -> usize {
        match self.kind.as_str() {
            "[hunt monster]" => 4,
            "[hunt enemy]" => 5,
            _ => 0,
        }
    }

    pub fn hunt_goal_total(&self) -> Option<i64> {
        let stride = self.hunt_stride();
        if stride == 0 || self.condition.is_empty() || self.condition.len() % stride != 0 {
            return None;
        }
        let mut total = 0i64;
        let mut index = stride - 1;
        while index < self.condition.len() {
            total += self.condition[index].max(0);
            index += stride;
        }
        Some(total)
    }

    /// Collection quests submit [item, quantity] pairs; id 0 is gold.
    pub fn seeking_requirements(&self) -> Vec<(i64, i64)> {
        if self.kind != "[seeking]" {
            return Vec::new();
        }
        let mut merged: HashMap<i64, i64> = HashMap::new();
        let mut index = 0;
        while index + 1 < self.condition.len() {
            *merged.entry(self.condition[index]).or_insert(0) += self.condition[index + 1].max(0);
            index += 2;
        }
        let mut rows: Vec<(i64, i64)> = merged.into_iter().collect();
        rows.sort_by_key(|(item, _)| *item);
        rows
    }

    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "grade": self.grade,
            "type": self.kind,
            "minLevel": self.min_level,
            "maxLevel": self.max_level,
            "supported": self.supported,
            "reason": self.reason,
            "rewardGrow": self.reward_grow,
            "rewardSecondGrow": self.reward_second_grow,
            "rewardExpertJob": self.reward_expert,
            "selectionCount": self.selection.len() / 2,
            "huntGoal": self.hunt_goal_total(),
            "requires": self.seeking_requirements().into_iter().map(|(item, quantity)| json!({"item": item, "quantity": quantity})).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub id: u32,
    pub kind: i64,
    pub stack_limit: i64,
    pub minimum_level: i64,
    pub equip_slot: i64,
}

impl Item {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            1 => "消耗品",
            2 => "材料",
            3 => "装备",
            4 => "武器/装扮类装备",
            5 => "任务物品",
            6 => "宠物/宠物蛋",
            7 => "时装",
            8 => "装扮",
            _ => "其它",
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "kindName": self.kind_name(),
            "stackLimit": self.stack_limit,
            "minimumLevel": self.minimum_level,
            "equipSlot": self.equip_slot,
        })
    }
}

struct Cache<T> {
    source: PathBuf,
    stamp: Option<std::time::SystemTime>,
    rows: Vec<T>,
}

static QUEST_CACHE: OnceLock<Mutex<Option<Cache<Quest>>>> = OnceLock::new();
static ITEM_CACHE: OnceLock<Mutex<Option<Cache<Item>>>> = OnceLock::new();

fn stamp_of(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn child_number(node: &super::xml::Node, key: &str) -> i64 {
    node.child_text(key)
        .and_then(|text| text.trim().parse::<i64>().ok())
        .unwrap_or_default()
}

fn child_string(node: &super::xml::Node, key: &str) -> String {
    node.child_text(key).unwrap_or_default().to_string()
}

fn int_list(node: &super::xml::Node, key: &str) -> Vec<i64> {
    let Some(list) = node.child(key) else {
        return Vec::new();
    };
    list.children
        .iter()
        .filter_map(|row| row.text.trim().parse::<i64>().ok())
        .collect()
}

fn load_quests(path: &Path) -> Result<Vec<Quest>, String> {
    let root = parse_file(path)?;
    let Some(container) = root.child("Quests") else {
        return Err("任务表缺少 Quests 节点。".to_string());
    };
    let mut rows = Vec::with_capacity(container.children.len());
    for node in container.children("Offline70QuestDefinitionFG") {
        rows.push(Quest {
            id: child_number(node, "Id").max(0) as u32,
            name: child_string(node, "Name"),
            grade: child_string(node, "Grade"),
            kind: child_string(node, "Type"),
            min_level: child_number(node, "MinLevel"),
            max_level: child_number(node, "MaxLevel"),
            supported: child_string(node, "Supported") == "true",
            reason: child_string(node, "Reason"),
            reward_grow: child_number(node, "RewardGrowType"),
            reward_second_grow: child_number(node, "RewardSecondGrow"),
            reward_expert: child_number(node, "RewardExpertJob"),
            condition: int_list(node, "Condition"),
            selection: int_list(node, "Selection"),
        });
    }
    Ok(rows)
}

fn load_items(path: &Path) -> Result<Vec<Item>, String> {
    let root = parse_file(path)?;
    let Some(container) = root.child("Items") else {
        return Err("物品表缺少 Items 节点。".to_string());
    };
    let mut rows = Vec::with_capacity(container.children.len());
    for node in container.children("Offline70EconomyItemDefinition") {
        rows.push(Item {
            id: child_number(node, "ItemId").max(0) as u32,
            kind: child_number(node, "Kind"),
            stack_limit: child_number(node, "StackLimit"),
            minimum_level: child_number(node, "MinimumLevel"),
            equip_slot: child_number(node, "EquipSlot"),
        });
    }
    Ok(rows)
}

/// Caches the parsed catalogue and re-reads it when the file's mtime changes, so regenerating the
/// tables in the server repository is picked up without restarting the launcher.
fn cached<T>(slot: &'static OnceLock<Mutex<Option<Cache<T>>>>, path: &Path, load: fn(&Path) -> Result<Vec<T>, String>) -> Result<Vec<T>, String>
where
    T: Clone,
{
    let guard = slot.get_or_init(|| Mutex::new(None));
    let mut cache = guard
        .lock()
        .map_err(|_| "任务/物品表缓存被占用。".to_string())?;
    let stamp = stamp_of(path);
    let fresh = cache
        .as_ref()
        .map(|value| value.source == path && value.stamp == stamp)
        .unwrap_or(false);
    if !fresh {
        *cache = Some(Cache {
            source: path.to_path_buf(),
            stamp,
            rows: load(path)?,
        });
    }
    Ok(cache
        .as_ref()
        .map(|value| value.rows.clone())
        .unwrap_or_default())
}

pub fn quests(path: &Path) -> Result<Vec<Quest>, String> {
    cached(&QUEST_CACHE, path, load_quests)
}

pub fn items(path: &Path) -> Result<Vec<Item>, String> {
    cached(&ITEM_CACHE, path, load_items)
}

fn matches(needle: &str, haystack: &str) -> bool {
    needle.is_empty() || haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub fn search_quests(path: &Path, query: &str, limit: usize, only_supported: bool) -> Result<Vec<Value>, String> {
    let mut rows: Vec<Value> = Vec::new();
    let numeric = query.trim().parse::<u32>().ok();
    for row in quests(path)? {
        if only_supported && !row.supported {
            continue;
        }
        let hit = numeric == Some(row.id) || matches(query, &row.name) || matches(query, &row.grade) || matches(query, &row.kind);
        if hit {
            rows.push(row.to_value());
            if rows.len() >= limit {
                break;
            }
        }
    }
    Ok(rows)
}

pub fn search_items(path: &Path, query: &str, limit: usize) -> Result<Vec<Value>, String> {
    let mut rows: Vec<Value> = Vec::new();
    let numeric = query.trim().parse::<u32>().ok();
    for row in items(path)? {
        if numeric == Some(row.id) || (numeric.is_none() && (matches(query, &row.id.to_string()) || matches(query, row.kind_name()))) {
            rows.push(row.to_value());
            if rows.len() >= limit {
                break;
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real tables when this machine has them, so the parser is proven against the files
    /// the launcher actually consumes (3212 quests, 39k item rows) instead of only fixtures.
    #[test]
    fn parses_the_delivery_catalogues() {
        let root = std::path::Path::new(r"D:8S3A3\Server-70s3a3\data");
        if !root.is_dir() {
            return;
        }
        let quests = quests(&root.join("quest-catalog.xml")).unwrap();
        assert!(quests.len() > 3000, "任务表行数 {}", quests.len());
        let promotion = quests.iter().find(|row| row.id == 806).expect("转职任务 806");
        assert_eq!(promotion.reward_grow, 1);
        assert!(promotion.name.contains('('));
        let hunt = quests.iter().find(|row| row.id == 4409).expect("狩猎任务 4409");
        assert_eq!(hunt.hunt_goal_total(), Some(20));
        let items = items(&root.join("item-metadata.xml")).unwrap();
        assert!(items.len() > 1000, "物品表行数 {}", items.len());
        assert!(items.iter().any(|row| row.id == 1000));
    }

    fn quest(kind: &str, condition: Vec<i64>) -> Quest {
        Quest {
            id: 1,
            name: "测试".to_string(),
            grade: "[grow]".to_string(),
            kind: kind.to_string(),
            min_level: 1,
            max_level: 40,
            supported: true,
            reason: String::new(),
            reward_grow: 0,
            reward_second_grow: 0,
            reward_expert: 0,
            condition,
            selection: Vec::new(),
        }
    }

    #[test]
    fn hunt_widths_match_the_server_engine() {
        assert_eq!(quest("[hunt monster]", vec![11, 0, 300, 3, 11, 1, 301, 2]).hunt_goal_total(), Some(5));
        assert_eq!(quest("[hunt enemy]", vec![11, 0, 300, 1, 4]).hunt_goal_total(), Some(4));
        assert_eq!(quest("[meet npc]", vec![19]).hunt_goal_total(), None);
    }

    #[test]
    fn seeking_rows_merge_duplicate_items_and_keep_gold() {
        let row = quest("[seeking]", vec![4170, 5, 4170, 5, 0, 2000]);
        let requirements = row.seeking_requirements();
        assert_eq!(requirements, vec![(0, 2000), (4170, 10)]);
    }

    #[test]
    fn search_hits_names_numbers_and_kinds() {
        let root = std::path::Path::new(r"D:8S3A3\Server-70s3a3\data");
        if !root.is_dir() {
            return;
        }
        let quests_path = root.join("quest-catalog.xml");
        let by_name = search_quests(&quests_path, "转职", 40, true).unwrap();
        assert!(by_name.iter().any(|row| row["id"] == json!(806)), "按名字没找到 806");
        let by_number = search_quests(&quests_path, "4409", 5, false).unwrap();
        assert_eq!(by_number[0]["id"], json!(4409));
        assert_eq!(by_number[0]["huntGoal"], json!(20));
        let awakening = search_quests(&quests_path, "觉醒", 60, true).unwrap();
        assert!(awakening.iter().any(|row| row["rewardSecondGrow"] == json!(1)), "没有带一次觉醒奖励的任务");
        let items = search_items(&root.join("item-metadata.xml"), "1000", 10).unwrap();
        assert_eq!(items[0]["id"], json!(1000));
        let by_kind = search_items(&root.join("item-metadata.xml"), "材料", 5).unwrap();
        assert!(by_kind.iter().all(|row| row["kind"] == json!(2)));
    }

    #[test]
    fn item_kind_labels_cover_the_grant_range() {
        assert_eq!(Item { id: 1, kind: 2, stack_limit: 1000, minimum_level: 1, equip_slot: 0 }.kind_name(), "材料");
        assert_eq!(Item { id: 1, kind: 99, stack_limit: 1, minimum_level: 1, equip_slot: 0 }.kind_name(), "其它");
    }
}
