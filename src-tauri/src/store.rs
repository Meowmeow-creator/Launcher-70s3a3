use std::collections::HashMap;

use serde_json::{json, Value};

use super::catalog::{self, Quest};
use super::config::Config;
use super::job;
use super::xml::{parse_file, Node};

pub const ROLES_FILE: &str = "roles-CX.xml";

/// 服务端 FG271 起把点券/代币券存在账号侧车档里（`Account-Cash-FG271.cs` 的 FileName），
/// 角色 economy 档里的同名字段退化成缓存：只有交易过一次的角色才会被刷成新数额，没动过的角色还留着旧值。
/// 所以显示余额要先看侧车档；它不存在就是并档之前的老存档（或还没起过新版服务端），照旧逐角色读。
const CASH_FILE: &str = "cash-account1-FG271.xml";

/// 商城余额：`key` 是 OfflineCash 或 BoundCash。
fn wallet_number(config: &Config, economy: &Node, key: &str) -> i64 {
    if let Some(wallet) = account_wallet(config) {
        if let Some(value) = wallet.child_number(key) {
            return value.max(0);
        }
    }
    economy.child_number(key).unwrap_or_default().max(0)
}

fn account_wallet(config: &Config) -> Option<Node> {
    let path = config.store.join(CASH_FILE);
    if !path.is_file() {
        return None;
    }
    parse_file(&path).ok()
}

/// 存档文件名规则照抄服务端 `Server-70s3a3/src/Tools/VmTestKit/Role-Scope-FG202.cs:18-20`。
/// id1 是多角色存档之前就交付的老角色，文件名里嵌了显示名 "yoyo"，服务端明确不许移动这些路径，
/// 所以只有它保留旧名字；后来的角色一律按 id 命名，客户端给的名字进不了文件路径。
fn economy_name(id: i64) -> String {
    if id == 1 { "economy-account1-yoyo-FG.xml".to_string() } else { format!("economy-account1-r{id}-FG.xml") }
}

fn progression_name(id: i64) -> String {
    if id == 1 { "progression-yoyo-FG.xml".to_string() } else { format!("progression-r{id}-FG.xml") }
}

fn skills_name(id: i64) -> String {
    if id == 1 { "skills-yoyo-EZ.xml".to_string() } else { format!("skills-r{id}-EZ.xml") }
}

fn field(node: &Node, key: &str) -> String {
    node.child_text(key).unwrap_or("0").trim().to_string()
}

fn number(node: &Node, key: &str) -> i64 {
    node.child_number(key).unwrap_or_default()
}

fn roster(roles: &Node) -> Vec<&Node> {
    roles
        .child("Roles")
        .map(|container| container.children("Offline70Role"))
        .unwrap_or_default()
}

fn text(node: &Node, key: &str) -> String {
    node.child_text(key).unwrap_or_default().trim().to_string()
}

/// 名册里某个角色的存档概况；从没进过游戏的角色没有 economy，返回 None 让调用方留空。
/// 最后一项是商城点券（服务端 GM `cash` 写进的就是它），按账号钱包读，元素缺失按 0。
fn outline(config: &Config, id: i64) -> Option<(i64, i64, i64, i64, i64)> {
    let economy_path = config.store.join(economy_name(id));
    let progression_path = config.store.join(progression_name(id));
    if !economy_path.is_file() || !progression_path.is_file() {
        return None;
    }
    let economy = parse_file(&economy_path).ok()?;
    let skills_path = config.store.join(skills_name(id));
    let spent_sp = if skills_path.is_file() {
        parse_file(&skills_path).map(|node| number(&node, "SpentSp")).unwrap_or_default()
    } else {
        0
    };
    Some((
        number(&economy, "Level"),
        number(&economy, "Gold"),
        number(&economy, "GrowType"),
        spent_sp,
        wallet_number(config, &economy, "OfflineCash"),
    ))
}

/// 名册全员的存档概况。服务端实时快照里 Roles 只有 roles-CX.xml 的记录等级（建角时写死 1），
/// 界面要显示真实等级/金币，就得按名册去拼每个角色自己的档。
pub fn outlines(config: &Config) -> Vec<Value> {
    let Ok(roles) = parse_file(&config.store.join(ROLES_FILE)) else {
        return Vec::new();
    };
    roster(&roles)
        .iter()
        .map(|row| role_row(config, *row))
        .collect()
}

/// 名册的一行加上该角色自己的存档，就是前端角色表要的一行；没建过档的角色数值留空。
fn role_row(config: &Config, row: &Node) -> Value {
    let id = text(row, "Id").parse::<i64>().unwrap_or_default();
    let role_job = text(row, "Job").parse::<i64>().unwrap_or_default();
    let (level, gold, grow, spent, cash) = outline(config, id).map_or(
        (None, None, None, None, None),
        |(level, gold, grow, spent, cash)| (Some(level), Some(gold), Some(grow), Some(spent), Some(cash)),
    );
    let first = grow.map(|value| value & 0x0f);
    let shown = |value: Option<i64>| value.map(|value| value.to_string()).unwrap_or_default();
    json!({
        "@id": text(row, "Id"),
        "@name": text(row, "Name"),
        "@job": text(row, "Job"),
        "@jobName": job::base_name(role_job),
        "@storedLevel": field(row, "Level"),
        "@level": shown(level),
        "@gold": shown(gold),
        "@offlineCash": shown(cash),
        "@grow": shown(grow),
        "@firstGrow": shown(first),
        "@firstGrowName": first.map(|value| job::advance_name(role_job, value).to_string()).unwrap_or_default(),
        "@sp": shown(spent),
        "@created": text(row, "CreatedUtc"),
    })
}

/// 觉醒名只在真的觉醒过（secondGrow!=0）时给出，任务表读不到就留空，不猜。
fn awakening_name(config: &Config, job_number: i64, first: i64, second: i64) -> String {
    if second == 0 || first == 0 {
        return String::new();
    }
    catalog::advancements_of(&config.quest_catalog(), job::family(job_number))
        .unwrap_or_default()
        .into_iter()
        .find(|row| row.grow == first)
        .map(|row| row.awakening)
        .unwrap_or_default()
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

/// 服务未启动时直接读某个角色的存档，产出的字段与 FG201 status 回执一致，前端只需一套渲染。
/// `role_id <= 0` 时取名册里的第一个角色。
pub fn read(config: &Config, role_id: i64) -> Result<Value, String> {
    let roles = parse_file(&config.store.join(ROLES_FILE))?;
    let rows = roster(&roles);
    let id = if role_id > 0 {
        role_id
    } else {
        rows.first()
            .and_then(|row| text(row, "Id").parse::<i64>().ok())
            .unwrap_or(1)
    };
    let economy_path = config.store.join(economy_name(id));
    let progression_path = config.store.join(progression_name(id));
    let mut missing: Vec<String> = Vec::new();
    for path in [&economy_path, &progression_path] {
        if !path.is_file() {
            missing.push(path.file_name().map(|value| value.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string()));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "角色 {id} 的存档还不存在：{}。请先用它进一次游戏（服务端会建它自己的档），再回来直读。",
            missing.join("、")
        ));
    }
    let economy = parse_file(&economy_path)?;
    let progression = parse_file(&progression_path)?;
    let skills_path = config.store.join(skills_name(id));
    let spent_sp = if skills_path.is_file() {
        number(&parse_file(&skills_path)?, "SpentSp")
    } else {
        0
    };
    let own = rows.iter().find(|row| text(row, "Id").parse::<i64>() == Ok(id));

    let grow = number(&economy, "GrowType");
    let first = grow & 0x0f;
    let second = (grow >> 4) & 0x07;
    let job_number = number(&economy, "Job");
    let inventory = economy
        .child("Inventory")
        .map(|container| container.children("Offline70EconomyEntry"))
        .unwrap_or_default();
    let mut inventory_items = 0i64;
    let mut inventory_rows = Vec::new();
    // 强化列只对物品类型 4/8 有意义，与服务端 GmSnapshotFG201 的判定逐字一致；
    // 存档里非装备也带 <Upgrade>0</Upgrade>，不查类型就会让"存档直读"比"服务端实时"多显示一列 0。
    let gear_kinds: HashMap<u32, i64> = catalog::items(&config.item_metadata())
        .unwrap_or_default()
        .into_iter()
        .map(|item| (item.id, item.kind))
        .collect();
    for row in &inventory {
        let list = number(row, "List");
        let quantity = number(row, "Quantity");
        if list != 2 && list != 3 {
            inventory_items += quantity;
            let item_id = number(row, "ItemId").max(0) as u32;
            let gear = matches!(gear_kinds.get(&item_id), Some(4) | Some(8));
            let upgrade = if gear { number(row, "Upgrade").to_string() } else { String::new() };
            let seed = match row.child_number("EquipmentSeed") {
                Some(value) if value >= 0 => value.to_string(),
                _ => String::new(),
            };
            inventory_rows.push(json!({
                "@list": list.to_string(),
                "@slot": number(row, "Slot").to_string(),
                "@item": item_id.to_string(),
                "@quantity": quantity.to_string(),
                "@upgrade": upgrade,
                "@seed": seed,
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
        "@roleName": own.map(|row| text(row, "Name")).unwrap_or_else(|| format!("角色 {id}")),
        "@job": field(&economy, "Job"),
        "@jobName": job::base_name(job_number),
        "@level": field(&economy, "Level"),
        "@experience": field(&economy, "Experience"),
        "@gold": field(&economy, "Gold"),
        "@offlineCash": wallet_number(config, &economy, "OfflineCash").to_string(),
        "@boundCash": wallet_number(config, &economy, "BoundCash").to_string(),
        "@questPoints": field(&economy, "QuestPoints"),
        "@bonusSp": field(&economy, "BonusSp"),
        "@spentSp": spent_sp.to_string(),
        "@grow": grow.to_string(),
        "@firstGrow": first.to_string(),
        "@firstGrowName": job::advance_name(job_number, first).to_string(),
        "@secondGrow": second.to_string(),
        "@awakeningName": awakening_name(config, job_number, first, second),
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
        "Roles": rows.iter().map(|row| role_row(config, *row)).collect::<Vec<_>>(),
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
        let store = std::path::Path::new(r"D:\70S3A3\local-run\DNF70-batch-test-FG");
        if !store.is_dir() {
            return;
        }
        let mut config = crate::config::Config::default();
        config.store = store.to_path_buf();
        // 角色 1：grow 17 → 一次转职 1（剑魂）、已一次觉醒。
        let snapshot = read(&config, 1).unwrap();
        assert_eq!(snapshot["@roleId"].as_str(), Some("1"));
        assert_eq!(snapshot["@roleName"].as_str(), Some("yoyo"));
        assert_eq!(snapshot["@job"].as_str(), Some("0"));
        assert_eq!(snapshot["@jobName"].as_str(), Some("鬼剑士"));
        assert_eq!(snapshot["@level"].as_str(), Some("70"));
        assert_eq!(snapshot["@grow"].as_str(), Some("17"));
        assert_eq!(snapshot["@firstGrow"].as_str(), Some("1"));
        assert_eq!(snapshot["@firstGrowName"].as_str(), Some("剑魂"));
        assert_eq!(snapshot["@secondGrow"].as_str(), Some("1"));
        if config.quest_catalog().is_file() {
            assert_eq!(snapshot["@awakeningName"].as_str(), Some("剑圣"));
        }
        assert_eq!(snapshot["ActiveQuests"].as_array().unwrap().len(), snapshot["@activeCount"].as_str().unwrap().parse::<usize>().unwrap());
        assert!(!snapshot["CompletedQuests"].as_array().unwrap().is_empty());
        // 名册列全账号里的角色，每行带上自己存档的数值；玩家建角会让名册变长，所以只锁形状不锁个数。
        let roles = snapshot["Roles"].as_array().unwrap();
        assert!(roles.len() >= 6, "名册只剩 {} 行：{roles:?}", roles.len());
        assert_eq!(roles[0]["@id"].as_str(), Some("1"));
        assert_eq!(roles[0]["@level"].as_str(), Some("70"));
        assert_eq!(roles[1]["@jobName"].as_str(), Some("男魔法师"));

        // 点券/代币券按账号钱包读：cash-account1-FG271.xml 在就以它为准，没有就照旧逐角色档读。
        let economy = parse_file(&config.store.join(economy_name(1))).unwrap();
        let wallet = account_wallet(&config);
        let shown = |name: &str| -> String {
            match wallet.as_ref().and_then(|node| node.child_number(name)) {
                Some(value) => value.to_string(),
                None => economy.child_text(name).unwrap_or("0").trim().to_string(),
            }
        };
        assert_eq!(snapshot["@offlineCash"].as_str(), Some(shown("OfflineCash").as_str()));
        assert_eq!(snapshot["@boundCash"].as_str(), Some(shown("BoundCash").as_str()));
        assert_eq!(snapshot["Roles"].as_array().unwrap()[0]["@offlineCash"].as_str(), snapshot["@offlineCash"].as_str());
        assert!(snapshot["@offlineCash"].as_str().unwrap().parse::<i64>().is_ok());

        // 装备条目带强化/品质，非装备（EquipmentSeed=-1）品质留空。
        let kinds: HashMap<u32, i64> = catalog::items(&config.item_metadata())
            .unwrap_or_default()
            .into_iter()
            .map(|item| (item.id, item.kind))
            .collect();
        let rows = snapshot["Inventory"].as_array().unwrap();
        let entries = economy
            .child("Inventory")
            .map(|container| container.children("Offline70EconomyEntry"))
            .unwrap_or_default();
        let mut seeded = 0usize;
        for entry in &entries {
            let list = number(entry, "List");
            let seed = entry.child_number("EquipmentSeed").unwrap_or(-1);
            let matched = rows.iter().find(|row| {
                row["@item"].as_str() == Some(number(entry, "ItemId").to_string().as_str())
                    && row["@slot"].as_str() == Some(number(entry, "Slot").to_string().as_str())
                    && row["@list"].as_str() == Some(list.to_string().as_str())
            });
            if list == 2 || list == 3 {
                assert!(matched.is_none(), "穿戴栏 {list} 不该出现在背包表里");
                continue;
            }
            let row = matched.unwrap_or_else(|| panic!("条目 {} 没有进背包表", number(entry, "ItemId")));
            let expected = if seed >= 0 { seed.to_string() } else { String::new() };
            assert_eq!(row["@seed"].as_str(), Some(expected.as_str()));
            // 强化列只对类型 4/8 存在，与服务端一致；非装备即使存档里写着 <Upgrade>0</Upgrade> 也必须留空。
            let item_id = number(entry, "ItemId").max(0) as u32;
            let gear = matches!(kinds.get(&item_id), Some(4) | Some(8));
            let expected_upgrade = if gear { number(entry, "Upgrade").to_string() } else { String::new() };
            assert_eq!(row["@upgrade"].as_str(), Some(expected_upgrade.as_str()), "条目 {} 强化列不对", item_id);
            if seed >= 0 {
                seeded += 1;
            }
        }
        assert_eq!(
            rows.iter().filter(|row| !row["@seed"].as_str().unwrap_or_default().is_empty()).count(),
            seeded,
            "带品质种子的条目数与存档不一致"
        );

        // 角色 2：job 8 男魔法师。等级/金币/转职进度会被玩家和 GM 改动（这个档已经练到 70 并转职），
        // 所以按 economy 元素核读数，只锁定身份与 grow 字节的解码规则。
        let second = read(&config, 2).unwrap();
        assert_eq!(second["@roleId"].as_str(), Some("2"));
        assert_eq!(second["@roleName"].as_str(), Some("1234"));
        assert_eq!(second["@job"].as_str(), Some("8"));
        assert_eq!(second["@jobName"].as_str(), Some("男魔法师"));
        let other = parse_file(&config.store.join(economy_name(2))).unwrap();
        let value = |name: &str| other.child_text(name).unwrap_or("0").trim().to_string();
        assert_eq!(second["@level"].as_str(), Some(value("Level").as_str()));
        assert_eq!(second["@gold"].as_str(), Some(value("Gold").as_str()));
        if wallet.is_some() {
            // 账号钱包已经生成：每个角色读到的点券必须是同一个数，这就是"点券通用"在直读侧的证据。
            assert_eq!(second["@offlineCash"].as_str(), snapshot["@offlineCash"].as_str(), "角色 2 与 yoyo 的点券不同源");
            assert_eq!(second["@boundCash"].as_str(), snapshot["@boundCash"].as_str(), "角色 2 与 yoyo 的代币券不同源");
        } else {
            assert_eq!(second["@offlineCash"].as_str(), Some(value("OfflineCash").as_str()));
            assert_eq!(second["@boundCash"].as_str(), Some(value("BoundCash").as_str()));
        }
        let second_grow = other.child_number("GrowType").unwrap_or_default();
        assert_eq!(second["@grow"].as_str(), Some(second_grow.to_string().as_str()));
        assert_eq!(second["@firstGrow"].as_str(), Some((second_grow & 0x0f).to_string().as_str()));
        assert_eq!(second["@secondGrow"].as_str(), Some(((second_grow >> 4) & 0x07).to_string().as_str()));
        assert_eq!(second["@firstGrowName"].as_str(), Some(job::advance_name(8, second_grow & 0x0f)));

        // 未转职的角色（名册里 grow 0 那批）转职名要留白，不猜分支。
        for row in roles.iter().filter(|row| row["@grow"].as_str() == Some("0")) {
            assert_eq!(row["@firstGrow"].as_str(), Some("0"), "{row:?}");
            assert_eq!(row["@firstGrowName"].as_str(), Some(""), "{row:?}");
        }

        // role_id<=0 落到名册第一个角色；读不到档的角色明确报错，不静默回落到 yoyo。
        assert_eq!(read(&config, 0).unwrap()["@roleId"].as_str(), Some("1"));
        let error = read(&config, 99).unwrap_err();
        assert!(error.contains("角色 99 的存档还不存在"), "{error}");
    }

    #[test]
    fn grow_bytes_decode_like_the_server() {
        assert_eq!(job::advance_name(0, 1), "剑魂");
        assert_eq!(job::advance_name(0, 0), "");
        let combined = 2 | (1 << 4);
        assert_eq!(combined & 0x0f, 2);
        assert_eq!((combined >> 4) & 0x07, 1);
    }

    #[test]
    fn store_file_names_follow_the_server_scope_rules() {
        assert_eq!(economy_name(1), "economy-account1-yoyo-FG.xml");
        assert_eq!(economy_name(4), "economy-account1-r4-FG.xml");
        assert_eq!(progression_name(1), "progression-yoyo-FG.xml");
        assert_eq!(progression_name(6), "progression-r6-FG.xml");
        assert_eq!(skills_name(1), "skills-yoyo-EZ.xml");
        assert_eq!(skills_name(2), "skills-r2-EZ.xml");
    }

    #[test]
    fn missing_elements_default_to_zero() {
        let root = crate::xml::parse_str("<Root><Level>70</Level></Root>").unwrap();
        assert_eq!(field(&root, "Level"), "70");
        assert_eq!(field(&root, "ExpertJobTypeFG183"), "0");
        assert_eq!(field(&root, "OfflineCash"), "0");
        assert_eq!(field(&root, "BoundCash"), "0");
        assert_eq!(number(&root, "Gold"), 0);
    }

    /// 可丢弃目录，不碰任何真实存档。
    #[test]
    fn mall_balance_follows_the_account_wallet_sidecar() {
        let dir = std::env::temp_dir().join(format!("qoder-fg271-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = crate::config::Config::default();
        config.store = dir.clone();
        let role = crate::xml::parse_str("<Offline70EconomyState><OfflineCash>5000</OfflineCash><BoundCash>7</BoundCash></Offline70EconomyState>").unwrap();
        assert!(account_wallet(&config).is_none());
        assert_eq!(wallet_number(&config, &role, "OfflineCash"), 5000, "并档前该角色读自己的点券");
        assert_eq!(wallet_number(&config, &role, "BoundCash"), 7);
        std::fs::write(
            dir.join(CASH_FILE),
            "<Offline70AccountCashStateFG271><Format>1</Format><AccountId>1</AccountId><Revision>1</Revision><OfflineCash>8000</OfflineCash><BoundCash>0</BoundCash></Offline70AccountCashStateFG271>",
        )
        .unwrap();
        assert_eq!(wallet_number(&config, &role, "OfflineCash"), 8000, "侧车档一出现就按账号钱包显示");
        assert_eq!(wallet_number(&config, &role, "BoundCash"), 0);
        assert_eq!(wallet_number(&config, &role, "Gold"), 0, "金币不是账号钱包，仍旧按角色档读");
        std::fs::remove_dir_all(&dir).ok();
    }
}
