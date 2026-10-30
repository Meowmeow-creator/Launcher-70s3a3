mod catalog;
mod config;
mod game;
mod gm;
mod hotkey;
mod job;
mod pvf;
mod service;
mod store;
mod xml;

use serde_json::{json, Value};

use config::Config;

/// 服务端 GM-Console-FG201.cs 认的操作名；这里只用于本地早报错，真正的边界服务端还会再校验一遍。
const GM_OPS: [&str; 16] = [
    "status",
    "role",
    "level",
    "cash",
    "gold",
    "fatigue",
    "item",
    "promote",
    "change-branch",
    "awaken",
    "quest-accept",
    "quest-complete",
    "dungeon-killall",
    "loot-pickall",
    "loot-autopick",
    "shutdown",
];

fn integer(params: &Value, key: &str, minimum: i64, maximum: i64, required: bool) -> Result<Option<String>, String> {
    let value = match params.get(key) {
        None | Some(Value::Null) => {
            if required {
                return Err(format!("缺少参数 {key}。"));
            }
            return Ok(None);
        }
        Some(found) => found,
    };
    let parsed = if let Some(text) = value.as_str() {
        text.trim().parse::<i64>().map_err(|_| format!("{key} 必须是整数。"))?
    } else if let Some(number) = value.as_i64() {
        number
    } else {
        return Err(format!("{key} 必须是整数。"));
    };
    if parsed < minimum || parsed > maximum {
        return Err(format!("{key} 必须在 {minimum}..{maximum} 之间（当前 {parsed}）。"));
    }
    Ok(Some(parsed.to_string()))
}

fn flag(params: &Value, key: &str, default: bool) -> Result<String, String> {
    let value = integer(params, key, 0, 1, false)?;
    Ok(value.unwrap_or(if default { "1" } else { "0" }.to_string()))
}

fn put(rows: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<String>) {
    if let Some(value) = value {
        rows.push((key, value));
    }
}

/// 界面把目标角色的 job 一起带过来，只用于本地收窄（服务端按自己绑定的角色判断，不接收这个属性）。
/// 读不到或超出九职业时返回 -1，调用方跳过收窄，交给服务端拒。
fn job_hint(params: &Value) -> i64 {
    match params.get("job") {
        Some(Value::String(text)) => text.trim().parse::<i64>().unwrap_or(-1),
        Some(value) => value.as_i64().unwrap_or(-1),
        None => -1,
    }
}

fn build_params(op: &str, params: &Value) -> Result<Vec<(&'static str, String)>, String> {
    if !params.is_object() {
        return Err("GM 参数必须是 JSON 对象。".to_string());
    }
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    match op {
        "status" | "awaken" | "fatigue" | "loot-pickall" | "shutdown" => {}
        "loot-autopick" => rows.push(("enable", flag(params, "enable", true)?)),
        "dungeon-killall" => put(&mut rows, "limit", integer(params, "limit", 1, 60, false)?),
        "role" => put(&mut rows, "roleId", integer(params, "roleId", 1, 65535, true)?),
        "level" => put(&mut rows, "to", integer(params, "to", 1, 70, true)?),
        "cash" | "gold" => put(&mut rows, "amount", integer(params, "amount", 1, 99_999_999, true)?),
        "promote" => put(&mut rows, "grow", integer(params, "grow", 1, 4, true)?),
        "change-branch" => {
            let grow = integer(params, "grow", 1, 4, true)?;
            // 换分支只在同职业内进行，非法分支服务端一定拒，这里按职业先挡一次并说清合法范围。
            let job_number = job_hint(params);
            if (0..job::count()).contains(&job_number) {
                let value = grow
                    .as_deref()
                    .and_then(|text| text.parse::<i64>().ok())
                    .unwrap_or_default();
                if !job::is_allowed(job_number, value) {
                    let legal = job::allowed(job_number)
                        .into_iter()
                        .map(|item| item.to_string())
                        .collect::<Vec<_>>()
                        .join("/");
                    return Err(format!(
                        "{}只有 grow {legal} 的转职分支（当前 grow {value}），换分支不会被接受。",
                        job::base_name(job_number)
                    ));
                }
            }
            put(&mut rows, "grow", grow);
        }
        "quest-accept" => put(&mut rows, "id", integer(params, "id", 1, 65535, true)?),
        "item" => {
            put(&mut rows, "item", integer(params, "item", 1, i64::from(u32::MAX), true)?);
            put(&mut rows, "count", integer(params, "count", 1, 1_000_000, true)?);
            // 强化/品质/再封印都是可选属性：一个都没填就保持老行为，只发 shortcut。
            let upgrade = integer(params, "upgrade", 0, 31, false)?;
            let seed = integer(params, "seed", 0, 2_147_483_647, false)?;
            let reseal = integer(params, "reseal", 0, 7, false)?;
            if upgrade.is_some() || seed.is_some() || reseal.is_some() {
                put(&mut rows, "upgrade", upgrade);
                put(&mut rows, "seed", seed);
                put(&mut rows, "reseal", reseal);
                rows.push(("sealed", flag(params, "sealed", false)?));
            }
            rows.push(("shortcut", flag(params, "shortcut", false)?));
        }
        "quest-complete" => {
            put(&mut rows, "id", integer(params, "id", 1, 65535, true)?);
            rows.push(("accept", flag(params, "accept", false)?));
            rows.push(("topup", flag(params, "topup", false)?));
            put(&mut rows, "selection", integer(params, "selection", 0, 65535, false)?);
        }
        other => return Err(format!("不支持的 GM 操作 {other}。")),
    }
    Ok(rows)
}

fn describe_error(error: String) -> String {
    if error.contains("FG201") || error.contains("TestItemService") {
        return format!("{error} 请把 Server-70s3a3 重新编译后再试（GM 接口是 FG201 新增的）。");
    }
    error
}

/// 转职按钮的候选项：先用任务表里该家族的"转职 + 一次觉醒"配对，读不到任务表才退回 job.rs 静态表。
fn promote_options(config: &Config, job_number: i64) -> Vec<Value> {
    let family = job::family(job_number);
    if !family.is_empty() {
        if let Ok(rows) = catalog::advancements_of(&config.quest_catalog(), family) {
            if !rows.is_empty() {
                return rows
                    .into_iter()
                    .map(|row| {
                        json!({
                            "@grow": row.grow.to_string(),
                            "@name": row.name,
                            "@awakening": row.awakening,
                            "@questId": row.quest_id.to_string(),
                        })
                    })
                    .collect();
            }
        }
    }
    job::allowed(job_number)
        .into_iter()
        .map(|grow| {
            json!({
                "@grow": grow.to_string(),
                "@name": job::advance_name(job_number, grow),
                "@awakening": "",
                "@questId": "",
            })
        })
        .collect()
}

/// 服务端把名册回成 `<Roles><Role/></Roles>`，XML→JSON 后是 `[{ "Role": [...] }]`；摊平成与存档直读
/// 一样的行形状，前端和补档逻辑就只需要处理一种结构。
fn flatten_roles(snapshot: &mut Value) {
    let nested = match snapshot.get("Roles") {
        Some(Value::Array(rows)) if rows.len() == 1 => match rows[0].get("Role") {
            Some(Value::Array(inner)) => Some(inner.clone()),
            _ => None,
        },
        _ => None,
    };
    let Some(rows) = nested else {
        return;
    };
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("Roles".to_string(), Value::Array(rows));
    }
}

/// 实时快照的 Roles 行只有 roles-CX.xml 里的记录等级（建角时写死 1），界面上的"等级/金币/转职"
/// 要按名册去拼每个角色自己的存档；存档直读模式本来就带这些字段，这里只补空着的。
fn merge_outlines(config: &Config, snapshot: &mut Value) {
    let Some(rows) = snapshot.get_mut("Roles").and_then(Value::as_array_mut) else {
        return;
    };
    if rows.iter().any(|row| !row.get("@level").and_then(Value::as_str).unwrap_or_default().is_empty()) {
        return;
    }
    let outlines = store::outlines(config);
    if outlines.is_empty() {
        return;
    }
    for row in rows.iter_mut() {
        let Some(object) = row.as_object_mut() else { continue };
        let id = object.get("@id").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(fill) = outlines.iter().find(|item| item.get("@id").and_then(Value::as_str) == Some(id.as_str())) else {
            continue;
        };
        for key in ["@level", "@gold", "@grow", "@firstGrow", "@firstGrowName", "@jobName", "@sp", "@offlineCash"] {
            if object.get(key).and_then(Value::as_str).unwrap_or_default().is_empty() {
                if let Some(value) = fill.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
        }
    }
}

/// 快照本身没有的东西由登录器补齐：转职候选项，以及界面选中的目标角色。
/// `@roleId` 是服务端当前绑定的角色，`@targetRoleId` 是用户在界面里选的那个；两者不一致时
/// 前端才会去发 `role` 请求重绑，或者在客户端占着服务端时把写入按钮禁用。
fn enrich(config: &Config, snapshot: &mut Value) {
    flatten_roles(snapshot);
    merge_outlines(config, snapshot);
    let Some(object) = snapshot.as_object_mut() else {
        return;
    };
    let read = |key: &str| -> String {
        object
            .get(key)
            .map(|value| match value {
                Value::String(text) => text.trim().to_string(),
                Value::Number(number) => number.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default()
    };
    let job_number = read("@job").parse::<i64>().unwrap_or(-1);
    let bound = read("@roleId");
    let options = json!(promote_options(config, job_number));
    let target = if config.gm_role > 0 { config.gm_role.to_string() } else { bound.clone() };
    let mut name = read("@roleName");
    if target != bound {
        name = object
            .get("Roles")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .map(|row| row.get("@id").and_then(Value::as_str).unwrap_or_default().to_string())
                    .position(|id| id == target)
                    .and_then(|index| rows[index].get("@name").and_then(Value::as_str))
                    .map(str::to_string)
            })
            .unwrap_or_default();
    }
    object.insert("PromoteOptions".to_string(), options);
    object.insert("@targetRoleId".to_string(), json!(target));
    object.insert("@targetRoleName".to_string(), json!(name));
}

/// 存档直读：读不出来时把原因并进提示，界面不能只剩一个空快照。
fn archive(config: &Config, note: String) -> (Value, &'static str, String) {
    match store::read(config, config.gm_role) {
        Ok(snapshot) => (snapshot, "存档直读", note),
        Err(error) => (Value::Null, "存档直读", format!("{note} {error}")),
    }
}

/// One refresh pass: live GM snapshot when the channel answers, archive read when it does not.
fn collect() -> Result<Value, String> {
    let config = config::load()?;
    let service_state = service::status(&config);
    let endpoint = gm::endpoint(&config);
    let (mut snapshot, mode, note) = match endpoint.as_ref() {
        Ok(handshake) if service_state["channelUp"] == json!(true) => match gm::request(&config, "status", &[]) {
            Ok(reply) => (
                gm::snapshot(&reply).cloned().unwrap_or(Value::Null),
                "服务端实时",
                format!("GM 通道 pid {}，角色 {}。", handshake.pid, handshake.role),
            ),
            Err(error) => archive(&config, format!("实时快照读取失败，已改用存档直读：{error}")),
        },
        Ok(handshake) => archive(
            &config,
            format!("频道服没有在 {} 监听，但 GM 握手文件仍在（pid {}），以存档为准。", config.channel_port, handshake.pid),
        ),
        Err(error) => archive(&config, error.clone()),
    };
    enrich(&config, &mut snapshot);
    Ok(json!({
        "config": serde_json::to_value(&config).map_err(|error| error.to_string())?,
        "service": service_state,
        "endpoint": endpoint.ok().map(|value| value.to_value()),
        "snapshot": snapshot,
        "mode": mode,
        "note": note,
        "gameProcesses": game::running(&config),
        "serverTime": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis() as u64)
            .unwrap_or_default(),
    }))
}

#[tauri::command]
fn read_config() -> Result<Value, String> {
    let config = config::load()?;
    Ok(json!({
        "config": serde_json::to_value(&config).map_err(|error| error.to_string())?,
        "defaultLaunchArguments": config::default_launch_arguments(config.entrance_port),
        "issue": config.validate(),
    }))
}

#[tauri::command]
fn save_config(config: Value) -> Result<Value, String> {
    let parsed: Config =
        serde_json::from_value(config).map_err(|error| format!("配置格式不正确：{error}"))?;
    if parsed.entrance_port < 17_000 || parsed.entrance_port > 17_999 {
        return Err("入口端口必须落在 17000..17999，服务端只接受这段测试端口。".to_string());
    }
    if parsed.channel_port == 0 {
        return Err("频道端口不能为空。".to_string());
    }
    config::save(&parsed)?;
    Ok(json!({ "message": "配置已保存。", "issue": parsed.validate() }))
}

#[tauri::command]
async fn refresh_state() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(collect)
        .await
        .map_err(|error| format!("状态检测任务失败：{error}"))?
}

#[tauri::command]
async fn service_start() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let config = config::load()?;
        let value = service::start(&config)?;
        Ok(json!({ "service": value, "state": collect()? }))
    })
    .await
    .map_err(|error| format!("启动任务失败：{error}"))?
}

#[tauri::command]
async fn service_stop() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let config = config::load()?;
        let value = service::stop(&config);
        Ok(json!({ "service": value, "state": collect()? }))
    })
    .await
    .map_err(|error| format!("停止任务失败：{error}"))?
}

#[tauri::command]
async fn client_launch() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let config = config::load()?;
        game::launch(&config)
    })
    .await
    .map_err(|error| format!("启动客户端失败：{error}"))?
}

#[tauri::command]
fn logs_refresh() -> Result<Value, String> {
    let config = config::load()?;
    Ok(service::logs(&config))
}

#[tauri::command]
async fn gm_command(op: String, params: Value) -> Result<Value, String> {
    let op = op.trim().to_string();
    if !GM_OPS.contains(&op.as_str()) {
        return Err(format!("不支持的 GM 操作 {op}。"));
    }
    let params = if params.is_null() { json!({}) } else { params };
    tauri::async_runtime::spawn_blocking(move || {
        let config = config::load()?;
        let rows = build_params(&op, &params)?;
        let pairs = rows.iter().map(|(key, value)| (*key, value.clone())).collect::<Vec<_>>();
        let reply = gm::request(&config, &op, &pairs).map_err(describe_error)?;
        let mut snapshot = gm::snapshot(&reply).cloned().unwrap_or(Value::Null);
        enrich(&config, &mut snapshot);
        Ok(json!({
            "op": op,
            "message": reply.get("@message").and_then(Value::as_str).unwrap_or("已完成。"),
            "snapshot": snapshot,
        }))
    })
    .await
    .map_err(|error| format!("GM 任务失败：{error}"))?
}

#[tauri::command]
fn set_hotkey(modifiers: u32, virtual_key: u32, label: String) -> Result<Value, String> {
    let mut config = config::load()?;
    if virtual_key == 0 && modifiers == 0 {
        config.hotkey_modifiers = 0;
        config.hotkey_vk = 0;
        config.hotkey_label = String::new();
        config.hotkey_cleared = true;
        config::save(&config)?;
        hotkey::clear();
        return Ok(json!({ "message": "快捷键已清除。", "registered": false }));
    }
    if virtual_key < 0x21 {
        return Err("请选择一个功能键或字母数字键（需要带 Ctrl/Alt/Shift/Win 之一）。".to_string());
    }
    if modifiers & 0xF == 0 {
        return Err("快捷键必须包含 Ctrl、Alt、Shift 或 Win 修饰键，否则会吃掉游戏里的普通按键。".to_string());
    }
    config.hotkey_modifiers = modifiers;
    config.hotkey_vk = virtual_key;
    config.hotkey_label = label;
    config.hotkey_cleared = false;
    config::save(&config)?;
    hotkey::request(modifiers, virtual_key);
    Ok(json!({ "message": "快捷键已登记（游戏有焦点时也能按）。", "registered": true }))
}

#[tauri::command]
fn quest_search(query: String, limit: u32, only_supported: bool) -> Result<Value, String> {
    let config = config::load()?;
    let rows = catalog::search_quests(&config.quest_catalog(), query.trim(), limit.clamp(1, 200) as usize, only_supported)?;
    Ok(json!({ "rows": rows, "catalog": config.quest_catalog().to_string_lossy() }))
}

#[tauri::command]
async fn item_names(ids: Vec<u32>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || pvf::names(&ids))
        .await
        .map_err(|error| format!("名字解析任务失败：{error}"))?
}

#[tauri::command]
fn item_search(query: String, limit: u32) -> Result<Value, String> {
    let config = config::load()?;
    let rows = catalog::search_items(&config.item_metadata(), query.trim(), limit.clamp(1, 200) as usize)?;
    Ok(json!({ "rows": rows, "catalog": config.item_metadata().to_string_lossy() }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// 单机界面只加载 exe 内嵌资源，不访问任何远程页面。朋友那边开着系统代理（clash/v2rayN 之类）时，
/// WebView2 会把内嵌页面用的 `http://tauri.localhost/` 也交给代理，代理拒连就是"嗯…无法访问此页面
/// ERR_CONNECTION_REFUSED"。这里让回环地址永远直连，并且保留用户自己已经设过的参数。
fn webview_arguments(existing: &str) -> String {
    const BYPASS: &str = "--proxy-bypass-list=<-loopback>;localhost:*;127.0.0.1:*;[::1]:*";
    if existing.contains("--proxy-bypass-list") {
        return existing.to_string();
    }
    if existing.trim().is_empty() {
        return BYPASS.to_string();
    }
    format!("{} {}", existing.trim(), BYPASS)
}

pub fn run() {
    let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    let arguments = webview_arguments(&existing);
    if arguments != existing {
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", arguments);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            hotkey::spawn(handle.clone());
            if let Ok(loaded) = config::load() {
                if loaded.hotkey_vk != 0 {
                    hotkey::request(loaded.hotkey_modifiers, loaded.hotkey_vk);
                }
            }
            let _ = handle;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            read_config,
            save_config,
            refresh_state,
            service_start,
            service_stop,
            client_launch,
            logs_refresh,
            gm_command,
            set_hotkey,
            quest_search,
            item_search,
            item_names
        ])
        .run(tauri::generate_context!())
        .expect("error while running the 70S3A3 launcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_bounded_before_touching_the_store() {
        assert!(build_params("level", &json!({"to": 71})).is_err());
        assert!(build_params("level", &json!({})).is_err());
        assert_eq!(build_params("level", &json!({"to": "70"})).unwrap()[0].1, "70");
        assert_eq!(build_params("promote", &json!({"grow": 4})).unwrap()[0].0, "grow");
        let complete = build_params("quest-complete", &json!({"id": 211})).unwrap();
        assert!(complete.contains(&("accept", "0".to_string())));
        assert!(complete.contains(&("topup", "0".to_string())));
        assert!(build_params("quest-complete", &json!({"id": 211, "selection": 1})).unwrap().contains(&("selection", "1".to_string())));
        assert!(build_params("dungeon-killall", &json!({})).unwrap().is_empty());
        assert_eq!(build_params("dungeon-killall", &json!({"limit": 5})).unwrap()[0].1, "5");
        assert!(build_params("fatigue", &json!({})).unwrap().is_empty());
        assert!(build_params("dungeon-killall", &json!({"limit": 61})).is_err());
        assert_eq!(build_params("loot-autopick", &json!({"enable": 0})).unwrap()[0].1, "0");
        assert_eq!(build_params("loot-autopick", &json!({})).unwrap()[0].1, "1");
    }

    #[test]
    fn unknown_operations_are_rejected_locally() {
        assert!(build_params("rm-rf", &json!({})).is_err());
        assert!(build_params("item", &json!({"item": 1000, "count": 0})).is_err());
        assert!(build_params("item", &json!({"item": 1000, "count": 1})).is_ok());
    }

    #[test]
    fn role_target_is_bounded_to_a_real_role_id() {
        assert!(build_params("role", &json!({"roleId": 0})).is_err());
        assert!(build_params("role", &json!({"roleId": 70000})).is_err());
        assert!(build_params("role", &json!({})).is_err());
        let rows = build_params("role", &json!({"roleId": 3})).unwrap();
        assert_eq!(rows, vec![("roleId", "3".to_string())]);
        assert!(GM_OPS.contains(&"role"));
        assert_eq!(GM_OPS.len(), 16);
    }

    /// 点券与金币都直接进服务端钱包，越界与缺参数都要在本地拒掉，别把 0 的请求发出去。
    #[test]
    fn cash_amount_is_bounded_before_the_server_sees_it() {
        assert!(build_params("cash", &json!({"amount": 0})).is_err());
        assert!(build_params("cash", &json!({"amount": 100_000_000})).is_err());
        assert!(build_params("cash", &json!({})).is_err());
        assert!(build_params("cash", &json!({"amount": -1})).is_err());
        assert_eq!(build_params("cash", &json!({"amount": "5000"})).unwrap(), vec![("amount", "5000".to_string())]);
        assert_eq!(build_params("cash", &json!({"amount": 99_999_999})).unwrap()[0].1, "99999999");
        assert!(GM_OPS.contains(&"cash"));
    }

    #[test]
    fn gold_amount_shares_the_cash_bounds() {
        assert!(build_params("gold", &json!({"amount": 0})).is_err());
        assert!(build_params("gold", &json!({"amount": 100_000_000})).is_err());
        assert!(build_params("gold", &json!({})).is_err());
        assert_eq!(build_params("gold", &json!({"amount": 500_000})).unwrap(), vec![("amount", "500000".to_string())]);
        assert!(GM_OPS.contains(&"gold"));
    }

    /// 内嵌页面走的是 http://tauri.localhost/；系统代理一拦就是"无法访问此页面"，
    /// 所以回环必须强制直连，同时不能覆盖用户自己设过的浏览器参数。
    #[test]
    fn loopback_never_goes_through_a_system_proxy() {
        let added = webview_arguments("");
        assert!(added.starts_with("--proxy-bypass-list=<-loopback>"), "{added}");
        assert!(added.contains("localhost:*") && added.contains("127.0.0.1:*") && added.contains("[::1]:*"));
        assert_eq!(
            webview_arguments("--disable-gpu"),
            format!("--disable-gpu {}", added)
        );
        assert_eq!(webview_arguments("--proxy-bypass-list=mine"), "--proxy-bypass-list=mine");
    }

    /// 换分支只在同职业内进行：暗夜使者（job 6）与男魔法师（job 8）只有 1/2 两个分支。
    #[test]
    fn change_branch_is_narrowed_to_the_roles_job() {
        assert!(build_params("change-branch", &json!({"grow": 3, "job": 6})).is_err());
        assert!(build_params("change-branch", &json!({"grow": 4, "job": "8"})).is_err());
        assert!(build_params("change-branch", &json!({"grow": 0, "job": 6})).is_err());
        assert!(build_params("change-branch", &json!({"job": 6})).is_err());
        let error = build_params("change-branch", &json!({"grow": 3, "job": 6})).unwrap_err();
        assert!(error.contains("暗夜使者只有 grow 1/2"), "{error}");
        // 合法组合只发 grow，job 是本地定位用的，服务端不收这个属性。
        assert_eq!(build_params("change-branch", &json!({"grow": 2, "job": 6})).unwrap(), vec![("grow", "2".to_string())]);
        assert_eq!(build_params("change-branch", &json!({"grow": 4, "job": 0})).unwrap(), vec![("grow", "4".to_string())]);
        // 认不出职业时不瞎收窄，交给服务端判断。
        assert!(build_params("change-branch", &json!({"grow": 3})).is_ok());
        assert!(build_params("change-branch", &json!({"grow": 3, "job": 42})).is_ok());
        assert!(GM_OPS.contains(&"change-branch"));
    }

    /// 强化/品质是可选属性：没填就维持只发 shortcut 的老行为，填了才连带 sealed 一起发。
    #[test]
    fn item_optional_attributes_only_travel_when_filled() {
        let plain = build_params("item", &json!({"item": 1000, "count": 1})).unwrap();
        assert_eq!(plain, vec![("item", "1000".to_string()), ("count", "1".to_string()), ("shortcut", "0".to_string())]);
        let enhanced = build_params("item", &json!({"item": 36201, "count": 1, "upgrade": 0})).unwrap();
        assert!(enhanced.contains(&("upgrade", "0".to_string())), "{enhanced:?} 里 upgrade=0 也要下发");
        assert!(enhanced.contains(&("sealed", "0".to_string())));
        assert!(enhanced.contains(&("shortcut", "0".to_string())));
        assert!(build_params("item", &json!({"item": 1000, "count": 1, "upgrade": 32})).is_err());
        assert!(build_params("item", &json!({"item": 1000, "count": 1, "seed": 2_147_483_648i64})).is_err());
        assert!(build_params("item", &json!({"item": 1000, "count": 1, "reseal": 8})).is_err());
        let full = build_params("item", &json!({"item": 36201, "count": 1, "upgrade": 12, "seed": 1_824_986_493, "reseal": 2, "sealed": 1})).unwrap();
        assert!(full.contains(&("upgrade", "12".to_string())));
        assert!(full.contains(&("seed", "1824986493".to_string())));
        assert!(full.contains(&("reseal", "2".to_string())));
        assert!(full.contains(&("sealed", "1".to_string())));
        // 只填 sealed 不算“要强化”，不下发强化属性。
        let sealed_only = build_params("item", &json!({"item": 1000, "count": 1, "sealed": 1})).unwrap();
        assert!(!sealed_only.contains(&("sealed", "1".to_string())));
        assert_eq!(sealed_only, vec![("item", "1000".to_string()), ("count", "1".to_string()), ("shortcut", "0".to_string())]);
    }

    #[test]
    fn enrich_adds_promote_options_and_the_target_role() {
        let config = Config::default();
        let mut snapshot = json!({"@job": "8", "@roleId": "2", "@roleName": "1234"});
        enrich(&config, &mut snapshot);
        assert_eq!(snapshot["@targetRoleId"].as_str(), Some("2"));
        assert_eq!(snapshot["@targetRoleName"].as_str(), Some("1234"));
        let options = snapshot["PromoteOptions"].as_array().unwrap();
        // 男魔法师在这套脚本里只有元素爆破师、冰结师两个分支。
        assert_eq!(options.len(), 2);
        assert_eq!(options[0]["@name"].as_str(), Some("元素爆破师"));
        assert_eq!(options[1]["@name"].as_str(), Some("冰结师"));
        if config.quest_catalog().is_file() {
            assert_eq!(options[0]["@questId"].as_str(), Some("4065"));
            assert_eq!(options[0]["@awakening"].as_str(), Some("魔皇"));
        }
        // 未知 job 不编造分支，只给空数组。
        let mut unknown = json!({"@job": "42"});
        enrich(&config, &mut unknown);
        assert_eq!(unknown["PromoteOptions"].as_array().unwrap().len(), 0);
        // 快照读失败时不炸，保持原样。
        let mut null = Value::Null;
        enrich(&config, &mut null);
        assert!(null.is_null());
    }

    /// 实时名册是 `[{ "Role": [...] }]` 且等级只有建角时的 1，要摊平并按各自存档补等级。
    /// 第二个角色取名册里实际存在的那一个：玩家在真机上建号删号，写死 id 3 的断言隔天就会漂成
    /// "该角色没有存档"，测试应当只依赖它真正想证明的东西。
    #[test]
    fn enrich_flattens_live_roles_and_fills_them_from_their_own_archive() {
        let config = Config::default();
        if !config.store.join(store::ROLES_FILE).is_file() {
            return;
        }
        let live = store::outlines(&config);
        let extra = live.iter().find(|row| row["@id"].as_str() != Some("1")).cloned();
        let mut roles = vec![json!({
            "@id": "1", "@name": "yoyo", "@job": "0", "@storedLevel": "1"
        })];
        if let Some(row) = &extra {
            roles.push(json!({
                "@id": row["@id"], "@name": row["@name"], "@job": row["@job"], "@storedLevel": "1"
            }));
        }
        let expected = roles.len();
        let mut snapshot = json!({
            "@job": "0",
            "@roleId": "1",
            "@roleName": "yoyo",
            "Roles": [{ "Role": roles }],
        });
        enrich(&config, &mut snapshot);
        let rows = snapshot["Roles"].as_array().unwrap();
        assert_eq!(rows.len(), expected, "名册没有摊平：{snapshot}");
        assert_eq!(rows[0]["@name"].as_str(), Some("yoyo"));
        assert_eq!(rows[0]["@level"].as_str(), Some("70"), "yoyo 的存档等级没补进来");
        if let Some(source) = &extra {
            let job_number = source["@job"]
                .as_str()
                .unwrap_or_default()
                .parse::<i64>()
                .unwrap_or(-1);
            assert_eq!(
                rows[1]["@jobName"].as_str(),
                Some(job::base_name(job_number).as_str()),
                "名册第二行的职业名没有按 job 枚举补上"
            );
            assert_eq!(rows[1]["@storedLevel"].as_str(), Some("1"));
        }
    }

    /// 界面选的角色和服务端绑定的角色不一致时，前端要靠这两个字段看出差别。
    #[test]
    fn enrich_keeps_target_and_bound_roles_apart() {
        let mut config = Config::default();
        config.gm_role = 3;
        let mut snapshot = json!({
            "@job": "0",
            "@roleId": "1",
            "@roleName": "yoyo",
            "Roles": [{"@id": "1", "@name": "yoyo"}, {"@id": "3", "@name": "2778"}],
        });
        enrich(&config, &mut snapshot);
        assert_eq!(snapshot["@roleId"].as_str(), Some("1"));
        assert_eq!(snapshot["@targetRoleId"].as_str(), Some("3"));
        assert_eq!(snapshot["@targetRoleName"].as_str(), Some("2778"));
        // 目标就是绑定角色时不额外造名字，沿用快照的 roleName。
        config.gm_role = 1;
        let mut same = json!({"@job": "0", "@roleId": "1", "@roleName": "yoyo", "Roles": [{"@id": "1", "@name": "yoyo"}]});
        enrich(&config, &mut same);
        assert_eq!(same["@targetRoleName"].as_str(), Some("yoyo"));
    }
}
