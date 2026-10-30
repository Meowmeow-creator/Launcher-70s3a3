use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CHANNEL_EXE: &str = "DNF70-Channel-FG.exe";
const ENTRANCE_EXE: &str = "DNF70-Entrance-FG.exe";
const CHANNEL_SCRIPT: &str = "DNF70-channel-info-CG.etc";
pub const DEFAULT_CHANNEL_PORT: u16 = 7002;
pub const DEFAULT_ENTRANCE_PORT: u16 = 17001;
/// 一键清图的全局快捷键出厂值：RegisterHotKey 的 MOD_CONTROL|MOD_ALT 加 'K'。
/// 游戏不占 Ctrl+Alt+ 字母，且必须带修饰键，免得吃掉普通按键。
pub const DEFAULT_HOTKEY_MODIFIERS: u32 = 0x0003;
pub const DEFAULT_HOTKEY_VIRTUAL_KEY: u32 = 0x4B;
pub const DEFAULT_HOTKEY_LABEL: &str = "Ctrl+Alt+K";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// 交付根目录，里面应当有 Server-70s3a3 与 local-run。
    pub root: PathBuf,
    pub store: PathBuf,
    pub logs: PathBuf,
    pub channel_port: u16,
    pub entrance_port: u16,
    /// 测试期默认关闭：秒怪会让"一键清图/掉落"这类验证失真。
    pub one_hit_kill: bool,
    pub client_dir: PathBuf,
    pub client_exe: String,
    pub launch_arguments: String,
    /// RegisterHotKey 的 fsModifiers/vk 组合，用于游戏前台时触发一键清图。
    pub hotkey_modifiers: u32,
    pub hotkey_vk: u32,
    pub hotkey_label: String,
    /// 玩家在设置页点过"清除"；没有这个标记就无法区分"老配置没这个字段"和"玩家主动清掉"，
    /// 出厂默认 Ctrl+Alt+K 会在每次启动时把清掉的绑定又加回来。
    pub hotkey_cleared: bool,
    /// GM 操作的目标角色；0 表示还没选，存档直读时落在名册第一个角色上。
    pub gm_role: i64,
}

impl Default for Config {
    fn default() -> Self {
        let root = guess_root().unwrap_or_else(|| PathBuf::from(r"D:\70S3A3"));
        Self {
            root: root.clone(),
            store: root.join("local-run").join("DNF70-batch-test-FG"),
            logs: root.join("local-run").join("logs"),
            channel_port: DEFAULT_CHANNEL_PORT,
            entrance_port: DEFAULT_ENTRANCE_PORT,
            one_hit_kill: false,
            hotkey_modifiers: DEFAULT_HOTKEY_MODIFIERS,
            hotkey_vk: DEFAULT_HOTKEY_VIRTUAL_KEY,
            hotkey_label: DEFAULT_HOTKEY_LABEL.to_string(),
            hotkey_cleared: false,
            client_dir: root.join("地下城与勇士"),
            client_exe: "DNF70_v50.exe".to_string(),
            launch_arguments: default_launch_arguments(DEFAULT_ENTRANCE_PORT),
            gm_role: 0,
        }
    }
}

pub fn default_launch_arguments(entrance_port: u16) -> String {
    // local-run/README.md §10 的已验证参数：第 3 字段是入口服端口，第 4、5 字段服务端只接受 "1"。
    format!("99?127.0.0.1?{entrance_port}?1?1?01?1?0?0?0?0?1?9n2b1c8r3w8y?0?0?19847")
}

impl Config {
    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("Server-70s3a3").join("build").join("stable")
    }

    pub fn channel_script(&self) -> PathBuf {
        self.root
            .join("Server-70s3a3")
            .join("data")
            .join(CHANNEL_SCRIPT)
    }

    pub fn quest_catalog(&self) -> PathBuf {
        self.root.join("Server-70s3a3").join("data").join("quest-catalog.xml")
    }

    pub fn item_metadata(&self) -> PathBuf {
        self.root
            .join("Server-70s3a3")
            .join("data")
            .join("item-metadata.xml")
    }

    pub fn gm_directory(&self) -> PathBuf {
        self.store.join("TestTools")
    }

    pub fn client_path(&self) -> PathBuf {
        self.client_dir.join(&self.client_exe)
    }

    /// 起服前必须成立的条件；返回第一条不满足的说明。
    pub fn validate(&self) -> Option<String> {
        if !self.bin_dir().is_dir() {
            return Some(format!("找不到服务端产物目录：{}", self.bin_dir().display()));
        }
        for name in [CHANNEL_EXE, ENTRANCE_EXE] {
            if !self.bin_dir().join(name).is_file() {
                return Some(format!("产物目录缺少 {name}，请先在 Server-70s3a3 里执行 python scripts/build.py --smoke。"));
            }
        }
        if !self.channel_script().is_file() {
            return Some(format!("缺少频道表：{}", self.channel_script().display()));
        }
        if self.store.file_name().and_then(|v| v.to_str()) != Some("DNF70-batch-test-FG") {
            return Some("存档目录名必须仍是 DNF70-batch-test-FG（服务端硬性要求可丢弃测试档）。".to_string());
        }
        if !self.store.is_dir() {
            return Some(format!("找不到存档目录：{}", self.store.display()));
        }
        if self.channel_port == self.entrance_port {
            return Some("频道端口与入口端口不能相同。".to_string());
        }
        None
    }
}

fn guess_root() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("DNF70_ROOT") {
        let path = PathBuf::from(from_env);
        if path.join("Server-70s3a3").is_dir() {
            return Some(path);
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        let mut directory = executable.parent().map(|value| value.to_path_buf());
        while let Some(candidate) = directory {
            if candidate.join("Server-70s3a3").is_dir() {
                return Some(candidate);
            }
            directory = candidate.parent().map(|value| value.to_path_buf());
        }
    }
    for candidate in [Path::new(r"D:\70S3A3"), Path::new(r"C:\70S3A3")] {
        if candidate.join("Server-70s3a3").is_dir() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

pub fn configuration_directory() -> Result<PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "无法确定当前用户的本地配置目录。".to_string())?;
    Ok(PathBuf::from(local_app_data).join("70S3A3单机启动器"))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(configuration_directory()?.join("config.json"))
}

pub fn load() -> Result<Config, String> {
    let path = config_path()?;
    if !path.is_file() {
        return Ok(Config::default());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("无法读取启动器配置：{error}"))?;
    let stored: Config = serde_json::from_slice(&bytes)
        .map_err(|error| format!("启动器配置无效：{error}"))?;
    Ok(merge_defaults(stored))
}

/// 老配置里没有的字段用默认值补齐，避免升级后要求用户重填路径。
fn merge_defaults(mut stored: Config) -> Config {
    let defaults = Config::default();
    if stored.root.as_os_str().is_empty() {
        stored.root = defaults.root;
    }
    if stored.store.as_os_str().is_empty() {
        stored.store = defaults.store;
    }
    if stored.logs.as_os_str().is_empty() {
        stored.logs = defaults.logs;
    }
    if stored.channel_port == 0 {
        stored.channel_port = defaults.channel_port;
    }
    if stored.entrance_port == 0 {
        stored.entrance_port = defaults.entrance_port;
    }
    if stored.client_dir.as_os_str().is_empty() {
        stored.client_dir = defaults.client_dir;
    }
    if stored.client_exe.is_empty() {
        stored.client_exe = defaults.client_exe;
    }
    if stored.launch_arguments.is_empty() {
        stored.launch_arguments = defaults.launch_arguments;
    }
    // 已经写盘的老配置里 hotkeyVk 是显式的 0（那会儿还没有出厂绑定），字段缺省补不上来；
    // 只要玩家没主动清除过，就补上默认组合，启动即能用快捷键清图。
    if !stored.hotkey_cleared && stored.hotkey_vk == 0 && stored.hotkey_modifiers == 0 {
        stored.hotkey_modifiers = defaults.hotkey_modifiers;
        stored.hotkey_vk = defaults.hotkey_vk;
        stored.hotkey_label = defaults.hotkey_label;
    }
    stored
}

pub fn save(config: &Config) -> Result<(), String> {
    let path = config_path()?;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|error| format!("无法创建配置目录：{error}"))?;
    }
    let json = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("无法生成配置：{error}"))?;
    std::fs::write(&path, json).map_err(|error| format!("无法保存配置：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_arguments_carry_the_entrance_port() {
        let arguments = default_launch_arguments(17001);
        let fields: Vec<&str> = arguments.split('?').collect();
        assert_eq!(fields[0], "99");
        assert_eq!(fields[1], "127.0.0.1");
        assert_eq!(fields[2], "17001");
        assert_eq!(fields[3], "1");
        assert_eq!(fields[4], "1");
    }

    #[test]
    fn validation_rejects_a_wrong_store_name() {
        let mut config = Config::default();
        config.store = PathBuf::from(r"D:\somewhere\other-store");
        let message = config.validate().unwrap_or_default();
        assert!(message.contains("DNF70-batch-test-FG") || message.contains("产物"));
    }

    #[test]
    fn config_round_trips_in_camel_case() {
        let json = serde_json::to_string(&Config::default()).unwrap();
        assert!(json.contains("channelPort"));
        assert!(json.contains("oneHitKill"));
        assert!(json.contains("gmRole"));
    }

    /// 老配置里没有 gmRole，补 0（0 是合法值：还没选目标角色）。
    #[test]
    fn an_old_config_without_gm_role_still_loads() {
        let parsed: Config = serde_json::from_str(r#"{"root":"D:\\70S3A3"}"#).unwrap();
        assert_eq!(parsed.gm_role, 0);
    }

    /// 快捷键的出厂值是 Ctrl+Alt+K：老配置里显式写着的 0（那会儿还没有默认绑定）会被补上，
    /// 而玩家主动点过"清除"的配置必须保持空，否则每次启动又把绑定加回来。
    #[test]
    fn a_legacy_config_without_a_binding_gets_the_default_hotkey() {
        let stored: Config = serde_json::from_str(
            r#"{"root":"D:\\70S3A3","hotkeyModifiers":0,"hotkeyVk":0,"hotkeyLabel":""}"#,
        )
        .unwrap();
        let merged = merge_defaults(stored);
        assert_eq!(merged.hotkey_vk, DEFAULT_HOTKEY_VIRTUAL_KEY);
        assert_eq!(merged.hotkey_modifiers, DEFAULT_HOTKEY_MODIFIERS);
        assert_eq!(merged.hotkey_label, DEFAULT_HOTKEY_LABEL);
    }

    #[test]
    fn an_explicitly_cleared_hotkey_stays_cleared() {
        let stored: Config = serde_json::from_str(
            r#"{"root":"D:\\70S3A3","hotkeyModifiers":0,"hotkeyVk":0,"hotkeyLabel":"","hotkeyCleared":true}"#,
        )
        .unwrap();
        let merged = merge_defaults(stored);
        assert_eq!(merged.hotkey_vk, 0);
        assert!(merged.hotkey_label.is_empty());
    }

    /// 玩家自己登记的组合不能被出厂值覆盖。
    #[test]
    fn a_registered_hotkey_survives_the_default_merge() {
        let stored: Config =
            serde_json::from_str(r#"{"root":"D:\\70S3A3","hotkeyModifiers":6,"hotkeyVk":120,"hotkeyLabel":"Shift+F9"}"#)
                .unwrap();
        let merged = merge_defaults(stored);
        assert_eq!(merged.hotkey_vk, 120);
        assert_eq!(merged.hotkey_modifiers, 6);
    }
}
