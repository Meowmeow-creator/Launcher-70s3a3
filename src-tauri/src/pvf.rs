use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Map, Value};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 27000;
const ROUTE: &str = "/Api/PvfUtiltiy/GetFileContents";
const STACK_INDEX: &str = "stackable/stackable.lst";
const EQUIP_INDEX: &str = "equipment/equipment.lst";

/// Names are resolved lazily per id and kept for the process lifetime; the two index files are
/// only fetched once (equipment.lst alone is ~3 MB).
#[derive(Default)]
struct Store {
    paths: HashMap<u32, String>,
    names: HashMap<u32, String>,
    missing: HashSet<u32>,
    indexes_read: bool,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

fn store() -> Result<std::sync::MutexGuard<'static, Store>, String> {
    STORE
        .get_or_init(|| Mutex::new(Store::default()))
        .lock()
        .map_err(|_| "PVF 名字缓存被占用。".to_string())
}

fn post(payload: &Value) -> Result<Value, String> {
    let body = payload.to_string();
    let request = format!(
        "POST {ROUTE} HTTP/1.1\r\nHost: {HOST}:{PORT}\r\nUser-Agent: 70s3a3-launcher\r\nContent-Type: application/json;charset=UTF-8\r\nOrigin: http://{HOST}:{PORT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect((HOST, PORT))
        .map_err(|error| format!("无法连接 pvfUtility（{HOST}:{PORT}）：{error}"))?;
    let timeout = Duration::from_secs(30);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("发送 PVF 请求失败：{error}"))?;
    stream.flush().map_err(|error| format!("发送 PVF 请求失败：{error}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|error| format!("读取 PVF 响应失败：{error}"))?;
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "PVF 响应没有头部结束标记。".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    if !head.contains("200") {
        return Err(format!("pvfUtility 返回了非 200 响应：{}", head.lines().next().unwrap_or_default()));
    }
    let mut payload_bytes = raw[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        payload_bytes = dechunk(&payload_bytes)?;
    }
    serde_json::from_slice::<Value>(&payload_bytes)
        .map_err(|error| format!("pvfUtility 返回了无效 JSON：{error}"))
}

fn dechunk(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut position = 0usize;
    loop {
        let Some(relative) = find(bytes, position, b"\r\n") else {
            break;
        };
        let size_text = String::from_utf8_lossy(&bytes[position..relative]).trim().to_string();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or_default(), 16)
            .map_err(|_| format!("PVF 分块长度无效：{size_text}"))?;
        if size == 0 {
            break;
        }
        let start = relative + 2;
        let end = start + size;
        if end > bytes.len() {
            return Err("PVF 分块响应被截断。".to_string());
        }
        output.extend_from_slice(&bytes[start..end]);
        position = end + 2;
        if position > bytes.len() {
            break;
        }
    }
    Ok(output)
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack[from..].windows(needle.len()).position(|window| window == needle).map(|at| at + from)
}

fn contents(response: &Value) -> Map<String, Value> {
    response
        .get("Data")
        .and_then(|data| data.get("FileContentData"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn text_of(files: &Map<String, Value>, path: &str) -> String {
    // GetFileContents keys are case sensitive, so try the requested spelling then the plain name.
    if let Some(found) = files.get(path).and_then(Value::as_str) {
        return found.to_string();
    }
    let lowered = path.to_lowercase();
    for (key, value) in files {
        if key.to_lowercase() == lowered {
            if let Some(text) = value.as_str() {
                return text.to_string();
            }
        }
    }
    String::new()
}

fn parse_index(text: &str, prefix: &str, into: &mut HashMap<u32, String>) {
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        let (id, path) = match line.split_once('\t') {
            Some(pair) => pair,
            None => continue,
        };
        let Ok(number) = id.trim().parse::<u32>() else { continue };
        let cleaned = path.trim().trim_matches('`');
        if cleaned.is_empty() || !cleaned.contains('.') {
            continue;
        }
        into.insert(number, format!("{prefix}/{cleaned}"));
    }
}

fn read_indexes(store: &mut Store) -> Result<(), String> {
    if store.indexes_read {
        return Ok(());
    }
    let response = post(&json!({ "FileList": [STACK_INDEX, EQUIP_INDEX] }))?;
    let files = contents(&response);
    parse_index(&text_of(&files, STACK_INDEX), "stackable", &mut store.paths);
    parse_index(&text_of(&files, EQUIP_INDEX), "equipment", &mut store.paths);
    if store.paths.is_empty() {
        return Err("pvfUtility 返回了空的物品索引。".to_string());
    }
    store.indexes_read = true;
    Ok(())
}

fn parse_name(text: &str) -> Option<String> {
    let at = text.find("[name]")?;
    for line in text[at + 6..].split('\n') {
        let trimmed = line.trim().trim_matches('`');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// id -> 中文名。已解析过的走缓存，缺的批量向 pvfUtility 取；返回附带 pvfOnline 供界面提示。
pub fn names(ids: &[u32]) -> Result<Value, String> {
    let mut guard = store()?;
    let mut resolved: Map<String, Value> = Map::new();
    let mut wanted: Vec<u32> = Vec::new();
    for id in ids {
        if let Some(found) = guard.names.get(id) {
            resolved.insert(id.to_string(), json!(found));
        } else if !guard.missing.contains(id) {
            wanted.push(*id);
        }
    }
    let mut error: Option<String> = None;
    if !wanted.is_empty() {
        if let Err(read) = read_indexes(&mut guard) {
            error = Some(read);
        } else {
            let mut paths: Vec<String> = Vec::new();
            let mut by_path: HashMap<String, Vec<u32>> = HashMap::new();
            for id in &wanted {
                match guard.paths.get(id) {
                    Some(path) => {
                        by_path.entry(path.clone()).or_default().push(*id);
                        if !paths.contains(path) {
                            paths.push(path.clone());
                        }
                    }
                    None => {
                        guard.missing.insert(*id);
                    }
                }
            }
            for chunk in paths.chunks(40) {
                let payload = json!({ "FileList": chunk });
                match post(&payload) {
                    Ok(response) => {
                        let files = contents(&response);
                        for path in chunk {
                            let name = parse_name(&text_of(&files, path));
                            let owners = by_path.get(path).cloned().unwrap_or_default();
                            match name {
                                Some(text) => {
                                    for id in owners {
                                        guard.names.insert(id, text.clone());
                                        resolved.insert(id.to_string(), json!(text));
                                    }
                                }
                                None => {
                                    for id in owners {
                                        guard.missing.insert(id);
                                    }
                                }
                            }
                        }
                    }
                    Err(read) => {
                        error = Some(read);
                        break;
                    }
                }
            }
        }
    }
    Ok(json!({
        "names": resolved,
        "pvfOnline": error.is_none(),
        "message": error.unwrap_or_default(),
        "known": guard.names.len(),
        "indexed": guard.paths.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_lines_become_prefixed_paths() {
        let mut paths = HashMap::new();
        parse_index("#PVF_File\r\n0\t`gold.stk`\r\n1000\t`food_strawberry.stk`\r\n3037\t`material/cubepiece_clear.stk`\r\n11\t\n", "stackable", &mut paths);
        assert_eq!(paths.get(&1000).map(String::as_str), Some("stackable/food_strawberry.stk"));
        assert_eq!(paths.get(&3037).map(String::as_str), Some("stackable/material/cubepiece_clear.stk"));
        assert!(!paths.contains_key(&11));
    }

    #[test]
    fn name_field_is_the_first_value_line() {
        assert_eq!(
            parse_name("x\r\n[name]\r\n\t`野草莓`\r\n[/name]\r\n").as_deref(),
            Some("野草莓")
        );
        assert_eq!(parse_name("[icon]\r\n1\r\n").as_deref(), None);
    }

    #[test]
    fn chunked_bodies_are_reassembled() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(body).unwrap(), b"Wikipedia".to_vec());
    }
}
