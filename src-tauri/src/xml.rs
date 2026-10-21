use serde_json::{Map, Value};
#[cfg(test)]
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn flag(&self, key: &str) -> bool {
        self.attr(key) == Some("1")
    }

    pub fn child(&self, key: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == key)
    }

    pub fn children(&self, key: &str) -> Vec<&Node> {
        self.children.iter().filter(|node| node.name == key).collect()
    }

    pub fn child_text(&self, key: &str) -> Option<&str> {
        self.child(key).map(|node| node.text.as_str())
    }

    pub fn child_number(&self, key: &str) -> Option<i64> {
        self.child(key)
            .and_then(|node| node.text.parse::<i64>().ok())
            .or_else(|| self.child(key).and_then(|node| hex_number(&node.text)))
    }

    /// Generic mapping used by the GM panels: attributes keep an "@" prefix, child elements are
    /// always arrays so the front end never has to branch on one-vs-many.
    pub fn to_value(&self) -> Value {
        if self.attrs.is_empty() && self.children.is_empty() {
            return Value::String(self.text.clone());
        }
        let mut object = Map::new();
        for (name, value) in &self.attrs {
            object.insert(format!("@{name}"), Value::String(value.clone()));
        }
        if !self.text.is_empty() {
            object.insert("#text".to_string(), Value::String(self.text.clone()));
        }
        for child in &self.children {
            let entry = object
                .entry(child.name.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(list) = entry.as_array_mut() {
                list.push(child.to_value());
            }
        }
        Value::Object(object)
    }
}

fn hex_number(text: &str) -> Option<i64> {
    let stripped = text.trim().trim_start_matches("0x").trim_start_matches("0X");
    if stripped.is_empty() || !stripped.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    i64::from_str_radix(stripped, 16).ok()
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn starts(&self, needle: &[u8]) -> bool {
        self.bytes[self.position..].starts_with(needle)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.position += 1;
        }
    }

    /// Stop at the first byte contained in `stops` (a byte set, not a sequence).
    fn take_until_any(&mut self, stops: &[u8]) -> &'a str {
        let start = self.position;
        while self.position < self.bytes.len() && !stops.contains(&self.bytes[self.position]) {
            self.position += 1;
        }
        slice(self.bytes, start, self.position)
    }

    /// Stop at the first occurrence of the `stop` sequence.
    fn take_until(&mut self, stop: &[u8]) -> &'a str {
        let start = self.position;
        while self.position < self.bytes.len() && !self.starts(stop) {
            self.position += 1;
        }
        slice(self.bytes, start, self.position)
    }

    fn expect(&mut self, token: &[u8]) -> Result<(), String> {
        if !self.starts(token) {
            return Err(format!(
                "XML 第 {} 字节附近期望 {}。",
                self.position,
                String::from_utf8_lossy(token)
            ));
        }
        self.position += token.len();
        Ok(())
    }
}

fn slice(bytes: &[u8], start: usize, end: usize) -> &str {
    std::str::from_utf8(&bytes[start..end]).unwrap_or_default().trim_matches('\u{feff}')
}

pub fn parse_file(path: &Path) -> Result<Node, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| format!("{} 不是 UTF-8 XML。", path.display()))?;
    parse_str(text).map_err(|error| format!("{}：{error}", path.display()))
}

/// The service publishes `service.xml` and each reply with a temp-file + replace, so a reader that
/// arrives mid-publish can see an empty file. Retry a few times before calling it a failure.
pub fn parse_file_settled(path: &Path) -> Result<Node, String> {
    let mut last = String::new();
    for attempt in 0..12 {
        match parse_file(path) {
            Ok(node) => return Ok(node),
            Err(error) => {
                last = error;
                if !last.contains("不是元素起点") && !last.contains("没有任何元素") {
                    return Err(last);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
    }
    Err(format!("{} 在重试后仍不可读：{last}", path.display()))
}

pub fn parse_str(input: &str) -> Result<Node, String> {
    // Generated tables in this delivery are inconsistent about a UTF-8 BOM; skip it before scanning.
    let body = input.trim_start_matches('\u{feff}');
    let mut cursor = Cursor {
        bytes: input.as_bytes(),
        position: input.len() - body.len(),
    };
    loop {
        cursor.skip_whitespace();
        if cursor.position >= cursor.bytes.len() {
            return Err("XML 没有任何元素。".to_string());
        }
        if cursor.starts(b"<?") {
            cursor.take_until(b"?>");
            cursor.expect(b"?>")?;
            continue;
        }
        if cursor.starts(b"<!--") {
            cursor.take_until(b"-->");
            cursor.expect(b"-->")?;
            continue;
        }
        if cursor.peek() == Some(b'&') {
            // Leading entity text is invalid before a root element; skip it as stray content.
            cursor.position += 1;
            continue;
        }
        if cursor.peek() == Some(b'<') && cursor.bytes[cursor.position + 1] != b'/' {
            return read_element(&mut cursor);
        }
        return Err(format!("XML 第 {} 字节处不是元素起点。", cursor.position));
    }
}

fn read_element(cursor: &mut Cursor) -> Result<Node, String> {
    cursor.expect(b"<")?;
    let raw_name = cursor.take_until_any(b" \t\r\n/>").trim();
    if raw_name.is_empty() {
        return Err("XML 元素名缺失。".to_string());
    }
    let mut node = Node {
        name: unescape(raw_name),
        ..Default::default()
    };
    loop {
        cursor.skip_whitespace();
        match cursor.peek() {
            Some(b'/') => {
                cursor.position += 1;
                cursor.expect(b">")?;
                return Ok(node);
            }
            Some(b'>') => {
                cursor.position += 1;
                break;
            }
            Some(_) => {
                let attribute = cursor.take_until_any(b"= \t\r\n").trim();
                if attribute.is_empty() {
                    return Err(format!("XML 元素 {} 内有空属性名。", node.name));
                }
                cursor.skip_whitespace();
                cursor.expect(b"=")?;
                cursor.skip_whitespace();
                let quote = cursor.peek().ok_or("XML 属性值缺失。")?;
                if quote != b'"' && quote != b'\'' {
                    return Err(format!("XML 属性 {attribute} 没有引号包裹。"));
                }
                cursor.position += 1;
                let raw_value = cursor.take_until_any(&[quote]);
                cursor.expect(&[quote])?;
                node.attrs
                    .push((unescape(attribute), unescape(raw_value.trim())));
            }
            None => return Err("XML 在属性处结束。".to_string()),
        }
    }
    loop {
        if cursor.position >= cursor.bytes.len() {
            return Err(format!("XML 元素 {} 没有结束标签。", node.name));
        }
        if cursor.starts(b"</") {
            cursor.position += 2;
            let closing = cursor.take_until(b">").trim();
            cursor.expect(b">")?;
            if closing != node.name {
                return Err(format!(
                    "XML 结束标签不匹配：期望 {}，实际 {closing}。",
                    node.name
                ));
            }
            node.text = node.text.trim().to_string();
            return Ok(node);
        }
        if cursor.starts(b"<!--") {
            cursor.take_until(b"-->");
            cursor.expect(b"-->")?;
            continue;
        }
        if cursor.starts(b"<![CDATA[") {
            cursor.position += 9;
            node.text.push_str(cursor.take_until(b"]]>"));
            cursor.expect(b"]]>")?;
            continue;
        }
        if cursor.peek() == Some(b'<') {
            let child = read_element(cursor)?;
            node.children.push(child);
            continue;
        }
        let start = cursor.position;
        while cursor.position < cursor.bytes.len() && cursor.peek() != Some(b'<') {
            cursor.position += 1;
        }
        node.text.push_str(&unescape(slice(cursor.bytes, start, cursor.position)));
    }
}

pub fn unescape(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'&' {
            // Copy whole characters, not bytes, so multi-byte Chinese text survives untouched.
            let character = input[index..]
                .chars()
                .next()
                .unwrap_or_default();
            output.push(character);
            index += character.len_utf8();
            continue;
        }
        let Some(end) = input[index..].find(';') else {
            output.push('&');
            index += 1;
            continue;
        };
        let entity = &input[index + 1..index + end];
        let decoded = match entity {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" => Some("'".to_string()),
            "nbsp" => Some(" ".to_string()),
            _ if entity.starts_with('#') => {
                let digits = &entity[1..];
                let code = digits
                    .strip_prefix("x")
                    .or(digits.strip_prefix("X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| digits.parse::<u32>().ok());
                code.and_then(|value| char::from_u32(value)).map(String::from)
            }
            _ => None,
        };
        match decoded {
            Some(text) => {
                output.push_str(&text);
                index += end + 1;
            }
            None => {
                output.push('&');
                index += 1;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_declaration_attributes_and_nested_text() {
        let source = "<?xml version='1.0' encoding='utf-8'?>\n<Root a=\"1\" b='2'><Item id=\"9\">甲&amp;乙</Item><Item id=\"10\"/></Root>";
        let root = parse_str(source).unwrap();
        assert_eq!(root.attr("a"), Some("1"));
        assert_eq!(root.attr("b"), Some("2"));
        assert_eq!(root.children("Item").len(), 2);
        assert_eq!(root.children("Item")[0].text, "甲&乙");
        assert_eq!(root.child_number("Missing"), None);
    }

    #[test]
    fn a_leading_byte_order_mark_is_skipped() {
        let root = parse_str("\u{feff}<?xml version='1.0' encoding='utf-8'?>\r\n<Root a=\"1\"><Id>7</Id></Root>").unwrap();
        assert_eq!(root.name, "Root");
        assert_eq!(root.attr("a"), Some("1"));
        assert_eq!(root.child_text("Id"), Some("7"));
    }

    #[test]
    fn rejects_mismatched_closing_tags() {
        assert!(parse_str("<A><B></A>").is_err());
    }

    #[test]
    fn gm_reply_maps_to_predictable_json() {
        let reply = parse_str(
            "<GMReply success=\"1\" message=\"完成\"><Snapshot level=\"70\"><ActiveQuests><Quest id=\"211\" name=\"暗恋\"/></ActiveQuests></Snapshot></GMReply>",
        )
        .unwrap();
        let value = reply.to_value();
        assert_eq!(value["@success"], json!("1"));
        assert_eq!(value["Snapshot"][0]["ActiveQuests"][0]["Quest"][0]["@id"], json!("211"));
    }

    #[test]
    fn unescapes_numeric_entities_and_leaves_plain_text() {
        assert_eq!(unescape("A&#65;B&#x41;"), "AAB A".replace(' ', ""));
        assert_eq!(unescape("鬼剑士"), "鬼剑士");
    }
}
