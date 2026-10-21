use serde_json::{json, Value};

use super::catalog::{self, Quest};
use super::config::Config;
use super::xml::{parse_file, Node};

pub const ECONOMY_FILE: &str = "economy-account1-yoyo-FG.xml";
pub const PROGRESSION_FILE: &str = "progression-yoyo-FG.xml";
pub const ROLES_FILE: &str = "roles-CX.xml";
pub const SKILLS_FILE: &str = "skills-yoyo-EZ.xml";

fn field(node: &Node, key: &str) -> String {
    node.child_text(key).unwrap_or("0").trim().to_string()
}

fn number(node: &Node, key: &str) -> i64 {
    node.child_number(key).unwrap_or_default()
}

pub fn grow_name(first: i64) -> &'static str {
    match first {
        1 => "剑魂",
        2 => "鬼泣",
        3 => "狂战士",
        4 => "阿修罗",
        _ => "未转职",
    }
}

fn list_numbers(node: &Node, wrapper: &str, item: &str) -> Vec<i64> {
    let Some(list) = node.child(wrapper) else {
        return Vec::new();
    };
    list.children(item)
        .iter()
        .filter_map(|row| row.text.trim().parse::<i64>().ok())
        .collect()
}

/// 服务未启动时直接读存档，产出的字段与 FG201 status 回执一致，前端只需一套渲染。
pub fn read(config: &Config) -> Result<Value, String> {
    let economy = parse_file(&config.store.join(ECONOMY_FILE))?;
    let progression = parse_file(&config.store.join(PROGRESSION_FILE))?;
    let roles = parse_file(&config.store.join(ROLES_FILE))?;
    let skills = config.store.join(SKILLS_FILE);
    let spent_sp = if skills.is_file() {
        number(&parse_file(&skills)?, "SpentSp")
    } else {
        0
    };

    let grow = number(&economy, "GrowType");
    let first = grow & 0x0f;
    let second = (grow >> 4) & 0x07;
    let role_rows = roles
        .child("Roles")
        .map(|container| container.children("Offline70Role"))
        .unwrap_or_default();
    let inventory = economy
        .child("Inventory")
        .map(|container| container.children("Offline70EconomyEntry"))
        .unwrap_or_default();
    let mut inventory_items = 0i64;
    let mut inventory_rows = Vec::new();
    for row in &inventory {
        let list = number(row, "List");
        let quantity = number(row, "Quantity");
        if list != 2 && list != 3 {
            inventory_items += quantity;
            inventory_rows.push(json!({
                "@list": list.to_string(),
                "@slot": number(row, "Slot").to_string(),
                "@item": number(row, "ItemId").to_string(),
                "@quantity": quantity.to_string(),
            }));
        }
    }

    let quest_catalog = config.quest_catalog();
    let lookup: Vec<Quest> = catalog::quests(&quest_catalog).unwrap_or_default();
    let find = |id: u32| lookup.iter().find(|row| row.id == id).cloned();
    let mut active_rows = Vec::new();
    if let Some(active) = progression.child("Active") {
        for row in active.children("Offline70QuestActiveFG") {
            let id = number(row, "Id") as u32;
            let counters: Vec<i64> = row
                .child("Progress")
                .map(|list| list.children("int"))
                .unwrap_or_default()
                .iter()
                .filter_map(|value| value.text.trim().parse::<i64>().ok())
                .collect();
            let brief = find(id);
            let recorded = counters.iter().sum::<i64>();
            let goal = brief
                .as_ref()
                .and_then(|value| value.hunt_goal_total())
                .unwrap_or_else(|| match counters.len() {
                    0 => 0,
                    _ => counters.len() as i64,
                });
            let (name, kind, grade, min_level, max_level, supported, reward_grow, reward_second_grow) = match &brief {
                Some(value) => (
                    value.name.clone(),
                    value.kind.clone(),
                    value.grade.clone(),
                    value.min_level.to_string(),
                    value.max_level.to_string(),
                    if value.supported { "1".to_string() } else { "0".to_string() },
                    value.reward_grow.to_string(),
                    value.reward_second_grow.to_string(),
                ),
                None => (
                    format!("任务 {id}"),
                    String::new(),
                    String::new(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                ),
            };
            active_rows.push(json!({
                "@id": id.to_string(),
                "@name": name,
                "@grade": grade,
                "@type": kind,
                "@minLevel": min_level,
                "@maxLevel": max_level,
                "@supported": supported,
                "@rewardGrow": reward_grow,
                "@rewardSecondGrow": reward_second_grow,
                "@recorded": recorded.to_string(),
                "@goal": goal.to_string(),
                "@counters": counters.iter().map(|value| value.to_string()).collect::<Vec<_>>().join("/"),
            }));
        }
    }

    let completed = list_numbers(&progression, "Completed", "unsignedShort");
    let completed_ids = completed
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<String>>()
        .join(",");

    Ok(json!({
        "@source": "存档直读",
        "@account": field(&economy, "AccountId"),
        "@roleId": field(&economy, "RoleId"),
        "@roleName": role_rows.first().and_then(|row| row.child_text("Name")).unwrap_or("yoyo").to_string(),
        "@job": field(&economy, "Job"),
        "@level": field(&economy, "Level"),
        "@experience": field(&economy, "Experience"),
        "@gold": field(&economy, "Gold"),
        "@questPoints": field(&economy, "QuestPoints"),
        "@bonusSp": field(&economy, "BonusSp"),
        "@spentSp": spent_sp.to_string(),
        "@grow": grow.to_string(),
        "@firstGrow": first.to_string(),
        "@firstGrowName": grow_name(first).to_string(),
        "@secondGrow": second.to_string(),
        "@expertJob": field(&progression, "ExpertJobTypeFG183"),
        "@fatigueRemaining": field(&economy, "FatigueRemaining"),
        "@savedTown": field(&economy, "SavedTown"),
        "@revision": field(&economy, "Revision"),
        "@live": "0",
        "@selected": "0",
        "@ready": "0",
        "@town": field(&economy, "SavedTown"),
        "@area": "0",
        "@dungeon": "0",
        "@inventoryRows": inventory_rows.len().to_string(),
        "@inventoryItems": inventory_items.to_string(),
        "@completedCount": completed.len().to_string(),
        "@activeCount": active_rows.len().to_string(),
        "Roles": role_rows.iter().map(|row| json!({
            "@id": field(row, "Id"),
            "@name": field(row, "Name"),
            "@job": field(row, "Job"),
            "@storedLevel": field(row, "Level"),
            "@created": field(row, "CreatedUtc"),
        })).collect::<Vec<_>>(),
        "ActiveQuests": active_rows,
        "CompletedQuests": [ { "@ids": completed_ids } ],
        "Inventory": inventory_rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the archive reader against the real disposable store when it is present.
    #[test]
    fn reads_the_delivery_store_shape() {
        let store = std::path::Path::new(r"D:8S3A3\local-run\isolated-test\gm201\DNF70-batch-test-FG");
        if !store.is_dir() {
            return;
        }
        let mut config = crate::config::Config::default();
        config.store = store.to_path_buf();
        let snapshot = read(&config).unwrap();
        assert_eq!(snapshot["@level"].as_str(), Some("70"));
        assert_eq!(snapshot["@firstGrowName"].as_str(), Some("鬼泣"));
        assert_eq!(snapshot["@secondGrow"].as_str(), Some("1"));
        assert_eq!(snapshot["ActiveQuests"].as_array().unwrap().len(), snapshot["@activeCount"].as_str().unwrap().parse::<usize>().unwrap());
        assert!(!snapshot["CompletedQuests"].as_array().unwrap().is_empty());
    }

    #[test]
    fn grow_bytes_decode_like_the_server() {
        assert_eq!(grow_name(1), "剑魂");
        assert_eq!(grow_name(0), "未转职");
        let combined = 2 | (1 << 4);
        assert_eq!(combined & 0x0f, 2);
        assert_eq!((combined >> 4) & 0x07, 1);
    }

    #[test]
    fn missing_elements_default_to_zero() {
        let root = crate::xml::parse_str("<Root><Level>70</Level></Root>").unwrap();
        assert_eq!(field(&root, "Level"), "70");
        assert_eq!(field(&root, "ExpertJobTypeFG183"), "0");
        assert_eq!(number(&root, "Gold"), 0);
    }
}
