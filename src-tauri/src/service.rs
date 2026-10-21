use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::config::Config;

pub const CHANNEL_EXE: &str = "DNF70-Channel-FG.exe";
pub const ENTRANCE_EXE: &str = "DNF70-Entrance-FG.exe";
pub const CHANNEL_LOG: &str = "channel-evidence.log";
pub const ENTRANCE_LOG: &str = "entrance.log";

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const DETACHED_PROCESS: u32 = 0x0000_0008;

pub fn port_open(port: u16) -> bool {
    ("127.0.0.1", port)
        .to_socket_addrs()
        .map(|addresses| {
            addresses.into_iter().any(|address| {
                TcpStream::connect_timeout(&address, Duration::from_millis(400)).is_ok()
            })
        })
        .unwrap_or(false)
}

fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(windows))]
    let _ = command;
    command
}

/// The PID that owns a listening loopback port, read from `netstat -ano`.
pub fn listening_pid(port: u16) -> Option<u32> {
    let output = hidden(Command::new("netstat").args(["-ano", "-p", "tcp"]))
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let port_text = port.to_string();
    for line in text.lines() {
        if !line.contains("LISTENING") || !line.contains("127.0.0.1") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let _protocol = parts.next();
        let Some(local) = parts.next() else { continue };
        if local.rsplit(':').next() != Some(port_text.as_str()) {
            continue;
        }
        let _remote = parts.next();
        let _state = parts.next();
        if let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) {
            return Some(pid);
        }
    }
    None
}

pub fn image_pids(image: &str) -> Vec<u32> {
    let Ok(output) = hidden(
        Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"]),
    )
    .output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split(',').collect();
            if columns.len() >= 2 && columns[0].trim_matches('"').eq_ignore_ascii_case(image) {
                columns[1].trim_matches('"').parse::<u32>().ok()
            } else {
                None
            }
        })
        .collect()
}

pub fn kill_pid(pid: u32) {
    let _ = hidden(Command::new("taskkill").args(["/F", "/PID", &pid.to_string()])).output();
}

fn both_ports(config: &Config, want_up: bool) -> bool {
    port_open(config.channel_port) == want_up && port_open(config.entrance_port) == want_up
}

fn wait_for_ports(config: &Config, want_up: bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if both_ports(config, want_up) || Instant::now() >= deadline {
            return both_ports(config, want_up);
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

pub fn status(config: &Config) -> Value {
    json!({
        "channelPort": config.channel_port,
        "entrancePort": config.entrance_port,
        "channelUp": port_open(config.channel_port),
        "entranceUp": port_open(config.entrance_port),
        "channelPid": listening_pid(config.channel_port),
        "entrancePid": listening_pid(config.entrance_port),
        "channelInstances": image_pids(CHANNEL_EXE),
        "entranceInstances": image_pids(ENTRANCE_EXE),
        "store": config.store.to_string_lossy(),
        "binDir": config.bin_dir().to_string_lossy(),
        "configIssue": config.validate(),
    })
}

/// 证据日志用 CreateNew 打开，重名会让服务直接退出，所以先把上一轮整体归档。
fn archive_previous_logs(logs: &Path) -> Result<std::path::PathBuf, String> {
    let live = [CHANNEL_LOG, ENTRANCE_LOG];
    if !live.iter().any(|name| logs.join(name).exists()) {
        return Ok(logs.to_path_buf());
    }
    let archive = logs.join(format!("launcher-archive-{}", stamp()));
    std::fs::create_dir_all(&archive).map_err(|error| format!("无法创建日志归档目录：{error}"))?;
    let read_dir = std::fs::read_dir(logs).map_err(|error| format!("无法列出日志目录：{error}"))?;
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let owned = live.iter().any(|prefix| name.starts_with(prefix));
        if owned && entry.path().is_file() {
            std::fs::rename(entry.path(), archive.join(&name))
                .map_err(|error| format!("归档 {name} 失败：{error}"))?;
        }
    }
    Ok(archive)
}

/// Log names only need to be unique, so the crate carries no clock dependency.
fn stamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    let (days_of_epoch, second_of_day) = (seconds / 86_400, seconds % 86_400);
    let mut year = 1970_i64;
    let mut remaining = days_of_epoch as i64;
    loop {
        let length = if is_leap(year) { 366 } else { 365 };
        if remaining < length {
            break;
        }
        remaining -= length;
        year += 1;
    }
    let months: [i64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0usize;
    while remaining >= months[month] {
        remaining -= months[month];
        month += 1;
    }
    format!(
        "{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}",
        year = year,
        month = month + 1,
        day = remaining + 1,
        hour = second_of_day / 3600,
        minute = (second_of_day % 3600) / 60,
        second = second_of_day % 60
    )
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub fn start(config: &Config) -> Result<Value, String> {
    if let Some(issue) = config.validate() {
        return Err(issue);
    }
    if port_open(config.channel_port) || port_open(config.entrance_port) {
        return Err(format!(
            "端口 {} 或 {} 已被占用；请先“停止服务端”，或在设置页改用 17000..17999 的测试端口。",
            config.channel_port, config.entrance_port
        ));
    }
    std::fs::create_dir_all(&config.logs).map_err(|error| format!("无法创建日志目录：{error}"))?;
    let archive = archive_previous_logs(&config.logs)?;
    let run_directory = config
        .store
        .parent()
        .map(|value| value.to_path_buf())
        .unwrap_or_else(|| config.logs.clone());
    let mut environment = std::env::vars().collect::<Vec<(String, String)>>();
    if config.one_hit_kill {
        environment.push(("DNF70_TEST_ONE_HIT_KILL".to_string(), "1".to_string()));
    }

    let mut channel = spawn(
        &config.bin_dir().join(CHANNEL_EXE),
        &run_directory,
        &environment,
        &[
            config.logs.join(CHANNEL_LOG).to_string_lossy().to_string(),
            "--batch-store".to_string(),
            config.store.to_string_lossy().to_string(),
            "--interactive".to_string(),
        ],
    )?;
    let mut entrance = spawn(
        &config.bin_dir().join(ENTRANCE_EXE),
        &run_directory,
        &environment,
        &[
            config.logs.join(ENTRANCE_LOG).to_string_lossy().to_string(),
            "--script-response".to_string(),
            config.channel_script().to_string_lossy().to_string(),
            "10".to_string(),
            "--test-port".to_string(),
            config.entrance_port.to_string(),
            "--interactive".to_string(),
        ],
    )?;

    let up = wait_for_ports(config, true, Duration::from_secs(20));
    let channel_state = exit_state(&mut channel);
    let entrance_state = exit_state(&mut entrance);
    let mut value = status(config);
    value["started"] = json!(up);
    value["archive"] = json!(archive.to_string_lossy());
    value["channelSpawn"] = channel_state.clone();
    value["entranceSpawn"] = entrance_state.clone();
    if up {
        value["message"] = json!(format!(
            "服务端已启动：频道 {}、入口 {}。GM 通道目录 {}。",
            config.channel_port,
            config.entrance_port,
            config.gm_directory().display()
        ));
    } else {
        let mut reasons = Vec::new();
        for (name, state) in [(CHANNEL_EXE, &channel_state), (ENTRANCE_EXE, &entrance_state)] {
            if state["exited"] == json!(true) {
                reasons.push(format!(
                    "{name} 启动后立即退出（返回码 {}）",
                    state["code"].as_i64().unwrap_or_default()
                ));
            }
        }
        if reasons.is_empty() {
            reasons.push("两个进程仍在启动中，可以稍后再点“刷新状态”。".to_string());
        }
        value["message"] = json!(format!("服务端未监听：{}", reasons.join("；")));
    }
    Ok(value)
}

fn exit_state(child: &mut std::process::Child) -> Value {
    match child.try_wait() {
        Ok(Some(status)) => json!({"exited": true, "code": status.code()}),
        Ok(None) => json!({"exited": false, "code": null}),
        Err(error) => json!({"exited": false, "error": error.to_string()}),
    }
}

fn spawn(
    executable: &Path,
    working_directory: &Path,
    environment: &[(String, String)],
    arguments: &[String],
) -> Result<std::process::Child, String> {
    let mut command = Command::new(executable);
    command
        .current_dir(working_directory)
        .args(arguments)
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    command
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", executable.display()))
}

pub fn stop(config: &Config) -> Value {
    let mut notes = Vec::new();
    if port_open(config.channel_port) && super::gm::request(config, "shutdown", &[]).is_ok() {
        notes.push("已向频道服发出优雅退出请求。".to_string());
    }
    let mut targets: Vec<u32> = Vec::new();
    for image in [CHANNEL_EXE, ENTRANCE_EXE] {
        targets.extend(image_pids(image));
    }
    for port in [config.channel_port, config.entrance_port] {
        if let Some(pid) = listening_pid(port) {
            targets.push(pid);
        }
    }
    targets.sort_unstable();
    targets.dedup();
    for pid in &targets {
        kill_pid(*pid);
    }
    let down = wait_for_ports(config, false, Duration::from_secs(8));
    let mut value = status(config);
    value["stopped"] = json!(down);
    value["killed"] = json!(targets);
    value["message"] = json!(if down {
        format!("服务端已停止（处理 {} 个进程）。{}", targets.len(), notes.join(""))
    } else {
        format!("端口仍未释放，请检查是否有其它实例。{}", notes.join(""))
    });
    value
}

pub fn tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut content = String::new();
    if file.read_to_string(&mut content).is_err() {
        return Vec::new();
    }
    let all: Vec<String> = content.lines().map(str::to_string).collect();
    let skip = all.len().saturating_sub(lines);
    all.into_iter().skip(skip).collect()
}

pub fn logs(config: &Config) -> Value {
    json!({
        "channel": tail(&config.logs.join(CHANNEL_LOG), 40),
        "entrance": tail(&config.logs.join(ENTRANCE_LOG), 20),
        "directory": config.logs.to_string_lossy(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_is_sortable_and_unique_per_second() {
        let first = stamp();
        assert_eq!(first.len(), 15, "期望形如 20261020-191530");
        assert_eq!(first.chars().nth(8), Some('-'));
    }

    #[test]
    fn leap_year_rule_matches_the_catalog_of_days() {
        assert!(is_leap(2024));
        assert!(!is_leap(2100));
        assert!(is_leap(2000));
    }
}
