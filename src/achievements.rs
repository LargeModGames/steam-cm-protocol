use std::collections::HashMap;

use crate::{
    connection::{Connection, ConnectionState},
    error::{Error, Result},
    friends::{ProtocolAchievement},
    protobuf::{CPlayerGetUserStatsRequest, CPlayerGetUserStatsResponse},
    service_method::{ServiceMethod, call_authed},
};

pub async fn get_player_achievements(
    connection: &Connection,
    state: &ConnectionState,
    appid: u32,
) -> Result<Vec<ProtocolAchievement>> {
    let steamid = state
        .steamid
        .ok_or(Error::MissingField("steamid not set in connection state"))?;

    let method = ServiceMethod::new("Player.GetUserStats#1");
    let request = CPlayerGetUserStatsRequest {
        steamid: Some(steamid),
        appid: Some(appid),
        ..Default::default()
    };

    let response: CPlayerGetUserStatsResponse =
        call_authed(connection, state, &method, &request).await?;

    let schema_bytes = response.schema.unwrap_or_default();
    if schema_bytes.is_empty() {
        return Ok(vec![]);
    }

    // Parse the binary KV schema to extract achievement definitions.
    let defs = match parse_achievement_schema(&schema_bytes) {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    if defs.is_empty() {
        return Ok(vec![]);
    }

    // Build unlock map: (stat_id, achievement_bit) → unlock_time
    let mut unlocked: HashMap<(u32, u32), u64> = HashMap::new();
    for stat in response.stats {
        let stat_id = stat.stat_id.unwrap_or(0);
        for ut in stat.unlock_times {
            let bit = ut.achievement_bit.unwrap_or(0);
            let time = ut.unlock_time.unwrap_or(0) as u64;
            if time > 0 {
                unlocked.insert((stat_id, bit), time);
            }
        }
    }

    let achievements = defs
        .into_iter()
        .map(|def| {
            let unlock_time = unlocked
                .get(&(def.stat_id, def.bit))
                .copied()
                .unwrap_or(0);
            ProtocolAchievement {
                apiname: def.internal_name,
                achieved: unlock_time > 0,
                unlocktime: unlock_time,
                name: def.display_name,
                description: def.description,
            }
        })
        .collect();

    Ok(achievements)
}

// ── Binary KeyValues parser ──────────────────────────────────────────────────

struct AchievementDef {
    stat_id: u32,
    bit: u32,
    internal_name: String,
    display_name: Option<String>,
    description: Option<String>,
}

/// Minimal representation of a binary KV node.
enum KVValue {
    Nested(Vec<(String, KVValue)>),
    Str(String),
    Int(i32),
    Other, // float, uint64, wstring, etc. — we don't need them for achievements
}

impl KVValue {
    fn as_nested(&self) -> Option<&Vec<(String, KVValue)>> {
        if let KVValue::Nested(n) = self { Some(n) } else { None }
    }

    fn as_str(&self) -> Option<&str> {
        if let KVValue::Str(s) = self { Some(s) } else { None }
    }

    fn as_int(&self) -> Option<i32> {
        if let KVValue::Int(i) = self { Some(*i) } else { None }
    }

    fn get(&self, key: &str) -> Option<&KVValue> {
        self.as_nested()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    fn read_i32_le(&mut self) -> Option<i32> {
        if self.remaining() < 4 {
            return None;
        }
        let bytes = &self.data[self.pos..self.pos + 4];
        self.pos += 4;
        Some(i32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn skip(&mut self, n: usize) -> bool {
        if self.remaining() < n {
            return false;
        }
        self.pos += n;
        true
    }

    fn read_null_string(&mut self) -> Option<String> {
        let start = self.pos;
        while self.pos < self.data.len() {
            if self.data[self.pos] == 0 {
                let s = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
                self.pos += 1; // consume null terminator
                return Some(s);
            }
            self.pos += 1;
        }
        None // unterminated string
    }

    /// Read a wstring: 4-byte length (UTF-16 code unit count) then that many u16 values.
    fn skip_wstring(&mut self) -> bool {
        let len = self.read_i32_le().unwrap_or(-1);
        if len < 0 {
            return false;
        }
        self.skip(len as usize * 2)
    }
}

fn parse_kv_children(r: &mut Reader<'_>) -> Option<Vec<(String, KVValue)>> {
    let mut children = Vec::new();
    loop {
        let type_byte = r.read_u8()?;
        if type_byte == 0x08 || type_byte == 0x0B {
            // end of nested block (0x08) or alt end (0x0B)
            break;
        }
        let key = r.read_null_string()?;
        let value = match type_byte {
            0x00 => {
                // nested
                KVValue::Nested(parse_kv_children(r)?)
            }
            0x01 => {
                // string
                KVValue::Str(r.read_null_string()?)
            }
            0x02 | 0x04 | 0x06 => {
                // int32 / pointer / color — all 4-byte LE
                KVValue::Int(r.read_i32_le()?)
            }
            0x03 => {
                // float — 4 bytes, not useful for achievements
                if !r.skip(4) { return None; }
                KVValue::Other
            }
            0x05 => {
                // wstring
                if !r.skip_wstring() { return None; }
                KVValue::Other
            }
            0x07 => {
                // uint64 — 8 bytes
                if !r.skip(8) { return None; }
                KVValue::Other
            }
            _ => {
                // unknown type — bail out, schema is malformed
                return None;
            }
        };
        children.push((key, value));
    }
    Some(children)
}

fn parse_kv(data: &[u8]) -> Option<KVValue> {
    let mut r = Reader::new(data);
    let type_byte = r.read_u8()?;
    if type_byte != 0x00 {
        // Root must be a nested node
        return None;
    }
    let _key = r.read_null_string()?; // root key (often empty)
    let children = parse_kv_children(&mut r)?;
    Some(KVValue::Nested(children))
}

/// Steam stores the stat type as a string ("4") OR occasionally as an int32.
/// Type 4 = achievement stat.
fn stat_type_is_achievement(stat_value: &KVValue) -> bool {
    if let Some(t) = stat_value.get("type") {
        if let Some(s) = t.as_str() {
            return s.parse::<u32>().unwrap_or(0) == 4;
        }
        if let Some(i) = t.as_int() {
            return i == 4;
        }
    }
    false
}

/// The display "name" and "desc" fields are language-keyed nested blocks:
///   display { name { english "First Blood" french "Premier Sang" } }
/// Fall back to plain-string form in case the game uses a simpler schema.
fn get_localized_string<'a>(node: &'a KVValue, key: &str) -> Option<&'a str> {
    let field = node.get(key)?;
    // Plain string (uncommon but guard against it)
    if let Some(s) = field.as_str() {
        return if s.is_empty() { None } else { Some(s) };
    }
    // Language-keyed nested node
    if field.as_nested().is_some() {
        // Prefer "english", then fall back to the first non-empty string child
        if let Some(eng) = field.get("english").and_then(|v| v.as_str())
            && !eng.is_empty() { return Some(eng); }
        if let Some(children) = field.as_nested() {
            for (_, v) in children {
                if let Some(s) = v.as_str()
                    && !s.is_empty() { return Some(s); }
            }
        }
    }
    None
}

/// Walk the parsed KV tree and extract achievement definitions.
/// Achievement stat groups have `type = 4` (stored as string).
fn extract_achievements(root: &KVValue) -> Vec<AchievementDef> {
    let mut defs = Vec::new();

    // Try root → "stats" first (standard layout)
    let stats_node = if let Some(s) = root.get("stats") {
        s
    } else if let Some(nested) = root.as_nested() {
        // Some schemas wrap the root in an extra level; look one level down.
        let mut found = None;
        for (_, v) in nested {
            if let Some(s) = v.get("stats") {
                found = Some(s);
                break;
            }
        }
        match found {
            Some(s) => s,
            None => return defs,
        }
    } else {
        return defs;
    };

    let stat_entries = match stats_node.as_nested() {
        Some(e) => e,
        None => return defs,
    };

    for (stat_key, stat_value) in stat_entries {
        let stat_id: u32 = match stat_key.parse() {
            Ok(id) => id,
            Err(_) => continue,
        };

        if !stat_type_is_achievement(stat_value) {
            continue;
        }

        let bits_node = match stat_value.get("bits").and_then(|b| b.as_nested()) {
            Some(b) => b,
            None => continue,
        };

        for (bit_key, bit_value) in bits_node {
            let bit: u32 = match bit_key.parse() {
                Ok(b) => b,
                Err(_) => continue,
            };

            let internal_name = match bit_value.get("name").and_then(|n| n.as_str()) {
                Some(n) if !n.is_empty() => n.to_owned(),
                _ => continue,
            };

            let display = bit_value.get("display");
            let display_name = display
                .and_then(|d| get_localized_string(d, "name"))
                .map(|s| s.to_owned());
            let description = display
                .and_then(|d| get_localized_string(d, "desc"))
                .map(|s| s.to_owned());

            defs.push(AchievementDef {
                stat_id,
                bit,
                internal_name,
                display_name,
                description,
            });
        }
    }

    defs
}

fn parse_achievement_schema(data: &[u8]) -> Result<Vec<AchievementDef>> {
    let root = parse_kv(data).ok_or_else(|| {
        Error::Transport("achievement schema binary KV parse failed".to_owned())
    })?;
    Ok(extract_achievements(&root))
}
