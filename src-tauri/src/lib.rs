mod catalog;
mod config;
mod game;
mod gm;
mod hotkey;
mod pvf;
mod service;
mod store;
mod xml;

use serde_json::{json, Value};

use config::Config;

const GM_OPS: [&str; 11] = [
    "status",
    "level",
    "item",
    "promote",
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

fn build_params(op: &str, params: &Value) -> Result<Vec<(&'static str, String)>, String> {
    if !params.is_object() {
        return Err("GM 参数必须是 JSON 对象。".to_string());
    }
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    match op {
        "status" | "awaken" | "loot-pickall" | "shutdown" => {}
        "loot-autopick" => rows.push(("enable", flag(params, "enable", true)?)),
        "dungeon-killall" => put(&mut rows, "limit", integer(params, "limit", 1, 60, false)?),
        "level" => put(&mut rows, "to", integer(params, "to", 1, 70, true)?),
        "promote" => put(&mut rows, "grow", integer(params, "grow", 1, 4, true)?),
        "quest-accept" => put(&mut rows, "id", integer(params, "id", 1, 65535, true)?),
        "item" => {
            put(&mut rows, "item", integer(params, "item", 1, i64::from(u32::MAX), true)?);
            put(&mut rows, "count", integer(params, "count", 1, 1_000_000, true)?);
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

/// One refresh pass: live GM snapshot when the channel answers, archive read when it does not.
fn collect() -> Result<Value, String> {
    let config = config::load()?;
    let service_state = service::status(&config);
    let endpoint = gm::endpoint(&config);
    let (snapshot, mode, note) = match endpoint.as_ref() {
        Ok(handshake) if service_state["channelUp"] == json!(true) => match gm::request(&config, "status", &[]) {
            Ok(reply) => (
                gm::snapshot(&reply).cloned().unwrap_or(Value::Null),
                "服务端实时",
                format!("GM 通道 pid {}，角色 {}。", handshake.pid, handshake.role),
            ),
            Err(error) => (
                store::read(&config).unwrap_or(Value::Null),
                "存档直读",
                format!("实时快照读取失败，已改用存档直读：{error}"),
            ),
        },
        Ok(handshake) => (
            store::read(&config).unwrap_or(Value::Null),
            "存档直读",
            format!("频道服没有在 {} 监听，但 GM 握手文件仍在（pid {}），以存档为准。", config.channel_port, handshake.pid),
        ),
        Err(error) => (
            store::read(&config).unwrap_or(Value::Null),
            "存档直读",
            error.clone(),
        ),
    };
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
        Ok(json!({
            "op": op,
            "message": reply.get("@message").and_then(Value::as_str).unwrap_or("已完成。"),
            "snapshot": gm::snapshot(&reply).cloned().unwrap_or(Value::Null),
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
pub fn run() {
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
}
