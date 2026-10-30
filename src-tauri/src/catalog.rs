use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use super::job;
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
    /// `<Job>[swordman]</Job>`，方括号已剥掉；`[all]` 保持 "all"。
    pub job: String,
    pub job_change: bool,
    pub reward_grow: i64,
    pub reward_second_grow: i64,
    pub reward_expert: i64,
    /// `<GrowType>`：转职/觉醒任务用它标自己属于哪个 grow 分支，通用任务是 -1。
    pub grow_type: i64,
    /// `<JobChangeStage>`：2 是觉醒链的步骤（结算步同时带 RewardSecondGrow=1）。
    pub job_change_stage: i64,
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
            "job": self.job,
            "jobChange": self.job_change,
            "rewardGrow": self.reward_grow,
            "rewardSecondGrow": self.reward_second_grow,
            "rewardExpertJob": self.reward_expert,
            "growType": self.grow_type,
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
            job: child_string(node, "Job").trim().trim_matches(|c| c == '[' || c == ']').to_string(),
            job_change: child_string(node, "JobChange") == "true",
            reward_grow: child_number(node, "RewardGrowType"),
            reward_second_grow: child_number(node, "RewardSecondGrow"),
            reward_expert: child_number(node, "RewardExpertJob"),
            grow_type: child_number(node, "GrowType"),
            job_change_stage: child_number(node, "JobChangeStage"),
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

/// 一个职业的一次转职分支：结算任务 + 该分支一次觉醒的结算任务。
#[derive(Clone, Debug, PartialEq)]
pub struct Advancement {
    pub grow: i64,
    pub name: String,
    pub quest_id: u32,
    pub awakening: String,
    pub awakening_quest_id: u32,
}

/// `转职 - 剑魂 (4/4)` → ("剑魂", 4, 4)。没有 `(n/m)` 步号的行（比如 "(后续确认)觉醒 - 龙斗士"）
/// 一律返回 None，避免把未定型的占位任务当成结算步。
fn chain_step<'a>(name: &'a str, prefix: &str) -> Option<(&'a str, i64, i64)> {
    let head = name
        .strip_prefix(prefix)?
        .trim_start_matches(|c: char| c == '-' || c == '－' || c.is_whitespace());
    let open = head.rfind('(')?;
    let body = head.get(open + 1..)?.strip_suffix(')')?;
    let (step, total) = body.split_once('/')?;
    Some((head[..open].trim(), step.trim().parse().ok()?, total.trim().parse().ok()?))
}

fn advancements_from(quests: &[Quest]) -> Vec<Vec<Advancement>> {
    let mut groups: Vec<Vec<Advancement>> = Vec::with_capacity(job::count() as usize);
    for index in 0..job::count() {
        let family = job::family(index);
        let mut rows = Vec::new();
        // 一次转职：JobChange=true 的结算步，RewardGrowType 即分支号。同一分支常有 legacy
        // `quest/cchange` 与现行 `n_quest` 两行（例：剑魂 806 与 7803），旧行 Supported=false，
        // 拿它当出处会让界面显示一个走任务线路必被拒的 ID，所以先取 Supported 的，再取 id 小的。
        let branch = |grow: i64| -> Option<(u32, String)> {
            let mut best: Option<(u32, String, bool)> = None;
            for row in quests.iter().filter(|row| {
                row.job == family
                    && row.job_change
                    && row.reward_grow == grow
                    && job::is_allowed(index, grow)
            }) {
                let Some((name, _, _)) = chain_step(&row.name, "转职") else {
                    continue;
                };
                let better = match &best {
                    None => true,
                    Some((id, _, chosen)) => if row.supported != *chosen { row.supported } else { row.id < *id },
                };
                if better {
                    best = Some((row.id, name.to_string(), row.supported));
                }
            }
            best.map(|(id, name, _)| (id, name))
        };
        // 一次觉醒：觉醒链结算步带 RewardSecondGrow=1，分支号在它自己的 <GrowType> 里
        // （觉醒任务行没有 RewardGrowType，RewardSecondGrow 只表示"这是一次觉醒"，恒为 1）。
        let awakening = |grow: i64| -> Option<(u32, String)> {
            let mut best: Option<(u32, String, bool)> = None;
            for row in quests.iter().filter(|row| {
                row.job == family
                    && !row.job_change
                    && row.job_change_stage == 2
                    && row.reward_second_grow == 1
                    && row.grow_type == grow
            }) {
                let Some((name, step, total)) = chain_step(&row.name, "觉醒") else {
                    continue;
                };
                if step != total {
                    continue;
                }
                let better = match &best {
                    None => true,
                    Some((id, _, chosen)) => if row.supported != *chosen { row.supported } else { row.id < *id },
                };
                if better {
                    best = Some((row.id, name.to_string(), row.supported));
                }
            }
            best.map(|(id, name, _)| (id, name))
        };
        for grow in job::allowed(index) {
            let Some((quest_id, name)) = branch(grow) else {
                continue;
            };
            let (awakening_quest_id, awakening_name) = awakening(grow).unwrap_or((0, String::new()));
            rows.push(Advancement {
                grow,
                name,
                quest_id,
                awakening: awakening_name,
                awakening_quest_id,
            });
        }
        groups.push(rows);
    }
    groups
}

/// 全部九职业的一次转职表，按 `job::family` 顺序、组内 grow 升序。
/// 登录器只按家族取用（`advancements_of`），整表视图留给需要一次列全职业的场景。
#[allow(dead_code)]
pub fn advancements(path: &Path) -> Result<Vec<Advancement>, String> {
    Ok(advancements_from(&quests(path)?).into_iter().flatten().collect())
}

/// 某个家族（`job::family` 的英文家族名）的一次转职表。
pub fn advancements_of(path: &Path, family: &str) -> Result<Vec<Advancement>, String> {
    let index = job::index_of_family(family).ok_or_else(|| format!("任务表里没有 {family} 这个职业。"))?;
    Ok(advancements_from(&quests(path)?)
        .into_iter()
        .nth(index as usize)
        .unwrap_or_default())
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
    /// the launcher actually consumes (3216 quests, 7259 item rows) instead of only fixtures.
    #[test]
    fn parses_the_delivery_catalogues() {
        let root = std::path::Path::new(r"D:\70S3A3\Server-70s3a3\data");
        if !root.is_dir() {
            return;
        }
        let quests = quests(&root.join("quest-catalog.xml")).unwrap();
        assert!(quests.len() > 3000, "任务表行数 {}", quests.len());
        let promotion = quests.iter().find(|row| row.id == 806).expect("转职任务 806");
        assert_eq!(promotion.reward_grow, 1);
        assert_eq!(promotion.job, "swordman");
        assert!(promotion.job_change);
        assert!(promotion.name.contains('('));
        let hunt = quests.iter().find(|row| row.id == 4409).expect("狩猎任务 4409");
        assert_eq!(hunt.hunt_goal_total(), Some(20));
        let items = items(&root.join("item-metadata.xml")).unwrap();
        assert!(items.len() > 1000, "物品表行数 {}", items.len());
        assert!(items.iter().any(|row| row.id == 1000));
    }

    /// 转职/觉醒表的口径直接跑真实 quest-catalog.xml 验证。
    #[test]
    fn advancements_pair_each_branch_with_its_awakening() {
        let path = std::path::Path::new(r"D:\70S3A3\Server-70s3a3\data\quest-catalog.xml");
        if !path.is_file() {
            return;
        }
        let swordman = advancements_of(path, "swordman").unwrap();
        assert_eq!(swordman.len(), 4, "鬼剑士分支 {}", swordman.len());
        // 806 是 legacy `quest/cchange/wm3.qst` 的重复行且 Supported=false，现行链的结算步是 7803；
        // 界面显示的转职任务 ID 必须是玩家真能走的那一条，否则按任务线路做会被拒。
        assert_eq!(
            swordman[0],
            Advancement {
                grow: 1,
                name: "剑魂".to_string(),
                quest_id: 7803,
                awakening: "剑圣".to_string(),
                awakening_quest_id: 977,
            }
        );
        let thief = advancements_of(path, "thief").unwrap();
        assert_eq!(
            thief.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            vec!["刺客", "死灵术士"]
        );
        let at_mage = advancements_of(path, "at mage").unwrap();
        assert_eq!(
            at_mage.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            vec!["元素爆破师", "冰结师"]
        );
        let priest = advancements_of(path, "priest").unwrap();
        assert_eq!(priest.len(), 4);
        assert_eq!(priest[3].name, "复仇者");
        assert_eq!(priest[3].awakening, "末日审判者");
        assert_eq!(priest[3].awakening_quest_id, 159);
        // 每个家族只留下 job::allowed 里的分支，合计 4*7+2+2=32 条，且分支名与 job.rs 静态表逐字相等。
        let all = advancements(path).unwrap();
        assert_eq!(all.len(), 32, "全职业转职分支 {}", all.len());
        let rows = quests(path).unwrap();
        for index in 0..job::count() {
            for row in advancements_of(path, job::family(index)).unwrap() {
                assert_eq!(row.name, job::advance_name(index, row.grow));
                assert!(!row.awakening.is_empty(), "{} grow={} 没有觉醒名", job::family(index), row.grow);
                assert!(
                    rows.iter().any(|quest| quest.id == row.quest_id && quest.supported),
                    "转职任务 {} 在服务端标记为未接入",
                    row.quest_id
                );
            }
        }
        assert!(advancements_of(path, "demonic swordman").is_err());
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
            job: "swordman".to_string(),
            job_change: false,
            reward_grow: 0,
            reward_second_grow: 0,
            reward_expert: 0,
            grow_type: 0,
            job_change_stage: 0,
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
        let root = std::path::Path::new(r"D:\70S3A3\Server-70s3a3\data");
        if !root.is_dir() {
            return;
        }
        let quests_path = root.join("quest-catalog.xml");
        let by_name = search_quests(&quests_path, "转职", 40, false).unwrap();
        assert!(by_name.iter().any(|row| row["id"] == json!(806)), "按名字没找到 806");
        // 806 那批 quest/cchange 行是旧重复链（Supported=false），当前 n_quest 链的结算步是 7803。
        let supported = search_quests(&quests_path, "转职", 40, true).unwrap();
        assert!(supported.iter().any(|row| row["id"] == json!(7803)), "可用转职链没找到 7803");
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
