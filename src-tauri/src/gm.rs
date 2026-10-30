use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::config::Config;
use super::xml::{parse_file_settled, parse_str};

pub const SERVICE_FILE: &str = "service.xml";
const REPLY_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub token: String,
    pub pid: u32,
    pub role: String,
    /// 服务端当前绑定的角色 id；旧服务端的握手文件里没有这个属性，读不到就是空串，
    /// 绑定态改由 status 快照的 @roleId 给出。
    pub role_id: String,
    /// 绑定角色的 job 枚举号，同样可能缺失。
    pub job: String,
    pub level: String,
    pub ready: bool,
    pub store: String,
    pub service: String,
}

impl Endpoint {
    pub fn to_value(&self) -> Value {
        json!({
            "pid": self.pid,
            "role": self.role,
            "roleId": self.role_id,
            "job": self.job,
            "level": self.level,
            "ready": self.ready,
            "store": self.store,
            "service": self.service,
            "tokenPresent": !self.token.is_empty(),
        })
    }
}

pub fn endpoint(config: &Config) -> Result<Endpoint, String> {
    let path = config.gm_directory().join(SERVICE_FILE);
    if !path.is_file() {
        return Err(format!(
            "服务端还没有开放 GM 通道（缺少 {}）。请先“一键启动服务端”。",
            path.display()
        ));
    }
    let node = parse_file_settled(&path)?;
    if node.name != "TestItemService" {
        return Err("GM 通道握手文件不是 TestItemService，服务端版本可能不匹配。".to_string());
    }
    let token = node.attr("token").unwrap_or_default().to_string();
    if token.len() != 32 {
        return Err("GM 通道令牌长度不正确，请确认服务端已包含 FG201 GM 接口。".to_string());
    }
    Ok(Endpoint {
        token,
        pid: node.attr("pid").and_then(|value| value.parse().ok()).unwrap_or(0),
        role: node.attr("role").unwrap_or_default().to_string(),
        role_id: node.attr("roleId").unwrap_or_default().to_string(),
        job: node.attr("job").unwrap_or_default().to_string(),
        level: node.attr("level").unwrap_or_default().to_string(),
        ready: node.flag("ready"),
        store: node.attr("store").unwrap_or_default().to_string(),
        service: node.attr("service").unwrap_or_default().to_string(),
    })
}

/// 32 lowercase hex characters, which is the receipt shape the server's Guid round-trip check accepts.
fn receipt() -> String {
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or_default();
    let counter = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0x9E37_79B9_85EB_AC87);
        NEXT.fetch_add(1, Ordering::Relaxed)
    };
    let mut state = tick ^ counter.rotate_left(23) ^ ((std::process::id() as u64) << 17);
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut text = String::with_capacity(32);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let temporary = path.with_extension("gm.tmp");
    std::fs::write(&temporary, content.as_bytes())
        .map_err(|error| format!("无法写入 GM 请求：{error}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("无法提交 GM 请求：{error}"))?;
    Ok(())
}

fn request_path(directory: &Path, value: &str) -> PathBuf {
    directory.join(format!("{value}.gm.xml"))
}

fn reply_path(directory: &Path, value: &str) -> PathBuf {
    directory.join(format!("{value}.gm.reply.xml"))
}

pub fn request(config: &Config, op: &str, params: &[(&str, String)]) -> Result<Value, String> {
    let handshake = endpoint(config)?;
    let directory = config.gm_directory();
    let id = receipt();
    let mut attributes = vec![
        ("version", "FG201".to_string()),
        ("token", handshake.token.clone()),
        ("receipt", id.clone()),
        ("op", op.to_string()),
    ];
    attributes.extend(params.iter().map(|(key, value)| (*key, value.clone())));
    let mut body = String::from("<GMRequest ");
    for (key, value) in &attributes {
        body.push_str(&format!("{key}=\"{}\" ", escape_attribute(value)));
    }
    body.pop();
    body.push_str("/>");
    write_atomic(&request_path(&directory, &id), &body)?;

    let reply = reply_path(&directory, &id);
    let deadline = Instant::now() + REPLY_TIMEOUT;
    loop {
        if reply.is_file() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = std::fs::remove_file(request_path(&directory, &id));
            return Err(format!(
                "服务端 {} 秒内没有回执（GM 通道 pid {}）。若角色正在副本中，部分操作会被拒绝。",
                REPLY_TIMEOUT.as_secs(),
                handshake.pid
            ));
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    let node = read_reply(&reply)?;
    let _ = std::fs::remove_file(&reply);
    if node.attr("receipt").unwrap_or_default() != id {
        return Err("GM 回执的回号与请求不一致，已忽略。".to_string());
    }
    let value = node.to_value();
    if node.flag("success") {
        Ok(value)
    } else {
        Err(node.attr("message").unwrap_or("GM 操作失败").to_string())
    }
}

/// The service publishes a reply with a temp-file + move, so a reader can catch it empty; re-read
/// the file (not the same bytes) until it parses or the budget runs out.
fn read_reply(path: &std::path::Path) -> Result<super::xml::Node, String> {
    let mut last = String::new();
    for attempt in 0..10 {
        match std::fs::read_to_string(path) {
            Ok(text) => match parse_str(&text) {
                Ok(node) => return Ok(node),
                Err(error) => last = error,
            },
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(Duration::from_millis(25 * (attempt + 1)));
    }
    Err(format!("GM 回执无法解析：{last}"))
}

/// Panels read the snapshot block as `reply.Snapshot[0]`.
pub fn snapshot(reply: &Value) -> Option<&Value> {
    reply
        .get("Snapshot")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
}


fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_are_lowercase_hex_and_unique() {
        let first = receipt();
        let second = receipt();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn attribute_values_cannot_break_out_of_quotes() {
        assert_eq!(escape_attribute("a\"b<c&d>e"), "a&quot;b&lt;c&amp;d&gt;e");
    }

    #[test]
    fn reply_parses_into_the_expected_json_shape() {
        let id = receipt();
        let text = format!(
            "<GMReply version=\"FG201\" receipt=\"{id}\" op=\"status\" success=\"1\" message=\"ok\"><Snapshot level=\"70\"/></GMReply>"
        );
        let node = parse_str(&text).unwrap();
        assert!(node.flag("success"));
        assert_eq!(node.attr("receipt"), Some(id.as_str()));
        assert_eq!(node.to_value()["Snapshot"][0]["@level"], json!("70"));
    }

    /// 握手文件里的 roleId/job 会被透出；旧服务端没有这两个属性时读成空串，不报错。
    #[test]
    fn handshake_reads_the_bound_role_identity() {
        let directory = std::env::temp_dir().join(format!("launcher-gm-{}", receipt()));
        let tools = directory.join("TestTools");
        std::fs::create_dir_all(&tools).unwrap();
        let path = tools.join(SERVICE_FILE);
        let mut config = Config::default();
        config.store = directory.clone();
        let token = "0123456789abcdef0123456789abcdef";

        std::fs::write(&path, format!("<TestItemService version=\"FG201\" token=\"{token}\" pid=\"4321\" role=\"1234\" roleId=\"2\" job=\"8\" level=\"3\" ready=\"1\"/>")).unwrap();
        let bound = endpoint(&config).unwrap();
        assert_eq!(bound.role_id, "2");
        assert_eq!(bound.job, "8");
        assert_eq!(bound.to_value()["roleId"], json!("2"));
        assert_eq!(bound.to_value()["job"], json!("8"));

        std::fs::write(&path, format!("<TestItemService version=\"FG201\" token=\"{token}\" pid=\"4321\" role=\"yoyo\" level=\"70\"/>")).unwrap();
        let legacy = endpoint(&config).unwrap();
        assert_eq!(legacy.role_id, "");
        assert_eq!(legacy.job, "");
        assert_eq!(legacy.to_value()["roleId"], json!(""));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
