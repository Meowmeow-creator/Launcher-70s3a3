use serde_json::{json, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use super::config::Config;
use super::service::{CREATE_NO_WINDOW, DETACHED_PROCESS};

pub fn validate(config: &Config) -> Result<std::path::PathBuf, String> {
    if config.client_dir.as_os_str().is_empty() || !config.client_dir.is_dir() {
        return Err(format!("找不到客户端目录：{}", config.client_dir.display()));
    }
    let executable = config.client_path();
    if !executable.is_file() {
        return Err(format!(
            "客户端目录里没有 {}，请在设置页改成实际可用的可执行文件。",
            config.client_exe
        ));
    }
    if !config.client_dir.join("Script.pvf").is_file() {
        return Err("客户端目录缺少 Script.pvf，客户端会在早期自杀退出。".to_string());
    }
    Ok(executable)
}

pub fn running(config: &Config) -> Vec<u32> {
    super::service::image_pids(&config.client_exe)
}

/// 客户端参数必须整串作为单个命令行参数传入；v50 版直接读第 3 字段的入口服端口。
pub fn launch(config: &Config) -> Result<Value, String> {
    let executable = validate(config)?;
    let entrance_open = super::service::port_open(config.entrance_port);
    let channel_open = super::service::port_open(config.channel_port);
    if !entrance_open || !channel_open {
        return Err(format!(
            "服务端还没起来（入口 {} {}，频道 {} {}），请先一键启动服务端。",
            config.entrance_port,
            if entrance_open { "已监听" } else { "未监听" },
            config.channel_port,
            if channel_open { "已监听" } else { "未监听" }
        ));
    }
    if !config.launch_arguments.contains(&config.entrance_port.to_string()) {
        return Err(format!(
            "启动参数第 3 字段（{}）与当前入口端口 {} 不一致，客户端会连到别的端口。",
            config.launch_arguments, config.entrance_port
        ));
    }
    let mut command = std::process::Command::new(&executable);
    command
        .current_dir(&config.client_dir)
        .arg(&config.launch_arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    let child = command.spawn().map_err(|error| {
        let hint = if matches!(error.raw_os_error(), Some(5) | Some(740) | Some(1223)) {
            " 该客户端可能需要管理员权限，请以管理员身份重新启动登录器。"
        } else {
            ""
        };
        format!("无法启动 {}：{error}.{hint}", executable.display())
    })?;
    let pid = child.id();
    std::mem::drop(child);
    Ok(json!({
        "pid": pid,
        "message": format!("客户端已启动（PID {pid}），请在频道列表里双击频道再点“游戏开始”。"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_reports_the_missing_executable() {
        let directory = std::env::temp_dir().join(format!("70s3-client-{}", std::process::id()));
        std::fs::create_dir_all(&directory).ok();
        let mut config = Config::default();
        config.client_dir = directory.clone();
        config.client_exe = "DNF70_missing.exe".to_string();
        let message = validate(&config).unwrap_err();
        assert!(message.contains("DNF70_missing.exe"));
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn launch_arguments_are_checked_against_the_entrance_port() {
        let directory = std::env::temp_dir().join(format!("70s3-client-args-{}", std::process::id()));
        std::fs::create_dir_all(&directory).ok();
        std::fs::write(directory.join("DNF70_local_test.exe"), []).unwrap();
        std::fs::write(directory.join("Script.pvf"), []).unwrap();
        let mut config = Config::default();
        config.client_dir = directory.clone();
        config.client_exe = "DNF70_local_test.exe".to_string();
        config.entrance_port = 17999;
        config.launch_arguments = "99?127.0.0.1?17001?1?1".to_string();
        let message = launch(&config).err().unwrap_or_default();
        assert!(message.contains("服务端") || message.contains("第 3 字段"));
        std::fs::remove_dir_all(&directory).ok();
    }
}
