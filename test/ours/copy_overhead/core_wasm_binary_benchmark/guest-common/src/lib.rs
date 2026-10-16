//! Core Wasm Direct Memory ABI 的 compact binary codec 與共用業務邏輯。
//!
//! 本檔案定義 host/guest 共用的資料型別、request offset-table layout、borrowed record/entry
//! views、compact responses，以及與 WCM 對齊的 parse、filter、format 實作和 ABI 記憶體工具。

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Crit = 4,
    Alert = 5,
    Emerg = 6,
}

impl LogLevel {
    /// 將 ABI 中的單一 byte 解析為日誌層級，拒絕未定義的值。
    pub fn from_byte(value: u8) -> Result<Self, CodecError> {
        match value {
            0 => Ok(Self::Debug),
            1 => Ok(Self::Info),
            2 => Ok(Self::Warn),
            3 => Ok(Self::Error),
            4 => Ok(Self::Crit),
            5 => Ok(Self::Alert),
            6 => Ok(Self::Emerg),
            _ => Err(CodecError::new("invalid log level")),
        }
    }

    /// 回傳用於層級比較與 binary encoding 的數值排名。
    pub fn rank(self) -> u8 {
        self as u8
    }

    /// 回傳 flat JSON format 使用的小寫層級名稱。
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 7] = ["debug", "info", "warn", "error", "crit", "alert", "emerg"];
        NAMES[usize::from(self.rank())]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub tags: Vec<(String, String)>,
    pub targettag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterResult {
    pub id: u64,
    pub keep: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecError {
    message: String,
}

impl CodecError {
    /// 以指定訊息建立 compact binary codec 錯誤。
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CodecError {
    /// 將 codec 錯誤的訊息寫入 formatter。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodecError {}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// 建立從輸入 slice 起點開始讀取的 owned-decoder reader。
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// 讀取固定長度的 slice 並前移游標，避免 offset overflow 與越界。
    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| CodecError::new("binary offset overflow"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| CodecError::new("truncated binary payload"))?;
        self.position = end;
        Ok(value)
    }

    /// 讀取一個 `u8`。
    fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.read_exact(1)?[0])
    }

    /// 以 little-endian 格式讀取一個 `u32`。
    fn read_u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(
            self.read_exact(4)?.try_into().expect("length checked"),
        ))
    }

    /// 以 little-endian 格式讀取一個 `u64`。
    fn read_u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(
            self.read_exact(8)?.try_into().expect("length checked"),
        ))
    }

    /// 讀取以 `u32` 編碼的項目數並轉為 `usize`。
    fn read_count(&mut self) -> Result<usize, CodecError> {
        usize::try_from(self.read_u32()?).map_err(|_| CodecError::new("count exceeds usize"))
    }

    /// 讀取 `u32 length + UTF-8 payload` 並建立 owned `String`。
    fn read_string(&mut self) -> Result<String, CodecError> {
        let length = self.read_count()?;
        let bytes = self.read_exact(length)?;
        let value =
            std::str::from_utf8(bytes).map_err(|_| CodecError::new("string is not UTF-8"))?;
        Ok(value.to_owned())
    }

    /// 確認 decoder 已完整消耗輸入，拒絕尾端多餘資料。
    fn finish(self) -> Result<(), CodecError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::new("trailing bytes in binary payload"))
        }
    }
}

/// 將 `usize` 檢查並轉為 little-endian `u32` 後附加到輸出。
fn push_u32(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(
        &u32::try_from(value)
            .expect("binary field exceeds u32")
            .to_le_bytes(),
    );
}

/// 將字串編碼為 `u32 length + UTF-8 payload` 後附加到輸出。
fn push_string(out: &mut Vec<u8>, value: &str) {
    push_u32(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

/// 依序編碼 tag 數量及每組 key/value 字串。
fn push_tags(out: &mut Vec<u8>, tags: &[(String, String)]) {
    push_u32(out, tags.len());
    for (key, value) in tags {
        push_string(out, key);
        push_string(out, value);
    }
}

/// 從 owned decoder 讀取 tag 數量與所有 key/value pairs。
fn read_tags(reader: &mut Reader<'_>) -> Result<Vec<(String, String)>, CodecError> {
    let count = reader.read_count()?;
    let mut tags = Vec::with_capacity(count);
    for _ in 0..count {
        tags.push((reader.read_string()?, reader.read_string()?));
    }
    Ok(tags)
}

const RECORD_HEADER_BYTES: usize = 4;
const RECORD_DESCRIPTOR_BYTES: usize = 8;
const ENTRY_HEADER_BYTES: usize = 8;
const ENTRY_DESCRIPTOR_BYTES: usize = 36;
const TAG_DESCRIPTOR_BYTES: usize = 16;

/// 在已配置的 request slice 指定位置寫入 little-endian `u32`。
fn put_u32(out: &mut [u8], position: usize, value: usize) {
    out[position..position + 4].copy_from_slice(
        &u32::try_from(value)
            .expect("direct-memory field exceeds u32")
            .to_le_bytes(),
    );
}

/// 在已配置的 request slice 指定位置寫入 little-endian `u64`。
fn put_u64(out: &mut [u8], position: usize, value: u64) {
    out[position..position + 8].copy_from_slice(&value.to_le_bytes());
}

/// 從指定位置讀取 little-endian `u32` offset/count 並轉為 `usize`。
fn get_u32(bytes: &[u8], position: usize) -> Result<usize, CodecError> {
    let raw = bytes
        .get(position..position + 4)
        .ok_or_else(|| CodecError::new("truncated direct-memory u32"))?;
    Ok(u32::from_le_bytes(raw.try_into().expect("length checked")) as usize)
}

/// 從指定位置讀取 little-endian `u64`。
fn get_u64(bytes: &[u8], position: usize) -> Result<u64, CodecError> {
    let raw = bytes
        .get(position..position + 8)
        .ok_or_else(|| CodecError::new("truncated direct-memory u64"))?;
    Ok(u64::from_le_bytes(raw.try_into().expect("length checked")))
}

/// 檢查 offset/length 後，借用 request payload 中的 UTF-8 字串。
fn view_string(bytes: &[u8], offset: usize, length: usize) -> Result<&str, CodecError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| CodecError::new("direct-memory string offset overflow"))?;
    std::str::from_utf8(
        bytes
            .get(offset..end)
            .ok_or_else(|| CodecError::new("direct-memory string is out of bounds"))?,
    )
    .map_err(|_| CodecError::new("direct-memory string is not UTF-8"))
}

/// 計算 parse request 的 header、record descriptors 與 payload 總長度。
pub fn parse_request_size(records: &[String]) -> usize {
    RECORD_HEADER_BYTES
        + RECORD_DESCRIPTOR_BYTES * records.len()
        + records.iter().map(String::len).sum::<usize>()
}

/// 將 record offset/length descriptors 與字串 payload 直接寫入指定 request slice。
pub fn write_parse_request(records: &[String], out: &mut [u8]) -> Result<(), CodecError> {
    let expected = parse_request_size(records);
    if out.len() != expected {
        return Err(CodecError::new("incorrect parse request buffer length"));
    }
    put_u32(out, 0, records.len());
    let mut payload = RECORD_HEADER_BYTES + RECORD_DESCRIPTOR_BYTES * records.len();
    for (index, record) in records.iter().enumerate() {
        let descriptor = RECORD_HEADER_BYTES + index * RECORD_DESCRIPTOR_BYTES;
        put_u32(out, descriptor, payload);
        put_u32(out, descriptor + 4, record.len());
        out[payload..payload + record.len()].copy_from_slice(record.as_bytes());
        payload += record.len();
    }
    Ok(())
}

pub struct RecordViews<'a> {
    bytes: &'a [u8],
    count: usize,
    index: usize,
}

/// 驗證 parse request layout，並建立不複製 payload 的 record iterator。
pub fn record_views(bytes: &[u8]) -> Result<RecordViews<'_>, CodecError> {
    let count = get_u32(bytes, 0)?;
    let descriptors_end = RECORD_HEADER_BYTES
        .checked_add(RECORD_DESCRIPTOR_BYTES * count)
        .ok_or_else(|| CodecError::new("record descriptor overflow"))?;
    if descriptors_end > bytes.len() {
        return Err(CodecError::new("truncated record descriptors"));
    }
    let mut payload_end = descriptors_end;
    for index in 0..count {
        let descriptor = RECORD_HEADER_BYTES + index * RECORD_DESCRIPTOR_BYTES;
        let offset = get_u32(bytes, descriptor)?;
        let length = get_u32(bytes, descriptor + 4)?;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| CodecError::new("record payload overflow"))?;
        if offset < descriptors_end || end > bytes.len() {
            return Err(CodecError::new("record payload is out of bounds"));
        }
        payload_end = payload_end.max(end);
    }
    if payload_end != bytes.len() {
        return Err(CodecError::new("trailing bytes in parse request"));
    }
    Ok(RecordViews {
        bytes,
        count,
        index: 0,
    })
}

impl<'a> Iterator for RecordViews<'a> {
    type Item = Result<&'a str, CodecError>;

    /// 借用下一筆 UTF-8 record；descriptor 或 payload 無效時回傳錯誤。
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.count {
            return None;
        }
        let descriptor = RECORD_HEADER_BYTES + self.index * RECORD_DESCRIPTOR_BYTES;
        self.index += 1;
        Some(get_u32(self.bytes, descriptor).and_then(|offset| {
            get_u32(self.bytes, descriptor + 4)
                .and_then(|length| view_string(self.bytes, offset, length))
        }))
    }
}

/// 計算 entries header、entry/tag descriptors 與所有字串 payload 的總長度。
pub fn entries_request_size(entries: &[LogEntry]) -> usize {
    let tag_count = entries.iter().map(|entry| entry.tags.len()).sum::<usize>();
    ENTRY_HEADER_BYTES
        + ENTRY_DESCRIPTOR_BYTES * entries.len()
        + TAG_DESCRIPTOR_BYTES * tag_count
        + entries
            .iter()
            .map(|entry| {
                entry.timestamp.len()
                    + entry.message.len()
                    + entry
                        .tags
                        .iter()
                        .map(|(key, value)| key.len() + value.len())
                        .sum::<usize>()
            })
            .sum::<usize>()
}

/// 將 entry/tag offset tables 與字串 payload 直接寫入指定 request slice。
pub fn write_entries_request(entries: &[LogEntry], out: &mut [u8]) -> Result<(), CodecError> {
    let expected = entries_request_size(entries);
    if out.len() != expected {
        return Err(CodecError::new("incorrect entries request buffer length"));
    }
    let total_tags = entries.iter().map(|entry| entry.tags.len()).sum::<usize>();
    put_u32(out, 0, entries.len());
    put_u32(out, 4, total_tags);
    let tag_table = ENTRY_HEADER_BYTES + ENTRY_DESCRIPTOR_BYTES * entries.len();
    let mut payload = tag_table + TAG_DESCRIPTOR_BYTES * total_tags;
    let mut tag_index = 0;

    for (index, entry) in entries.iter().enumerate() {
        let descriptor = ENTRY_HEADER_BYTES + index * ENTRY_DESCRIPTOR_BYTES;
        put_u64(out, descriptor, entry.id);
        put_u32(out, descriptor + 8, payload);
        put_u32(out, descriptor + 12, entry.timestamp.len());
        out[payload..payload + entry.timestamp.len()].copy_from_slice(entry.timestamp.as_bytes());
        payload += entry.timestamp.len();
        out[descriptor + 16] = entry.level.rank();
        out[descriptor + 17..descriptor + 20].fill(0);
        put_u32(out, descriptor + 20, payload);
        put_u32(out, descriptor + 24, entry.message.len());
        out[payload..payload + entry.message.len()].copy_from_slice(entry.message.as_bytes());
        payload += entry.message.len();
        put_u32(out, descriptor + 28, tag_index);
        put_u32(out, descriptor + 32, entry.tags.len());

        for (key, value) in &entry.tags {
            let tag = tag_table + tag_index * TAG_DESCRIPTOR_BYTES;
            put_u32(out, tag, payload);
            put_u32(out, tag + 4, key.len());
            out[payload..payload + key.len()].copy_from_slice(key.as_bytes());
            payload += key.len();
            put_u32(out, tag + 8, payload);
            put_u32(out, tag + 12, value.len());
            out[payload..payload + value.len()].copy_from_slice(value.as_bytes());
            payload += value.len();
            tag_index += 1;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct TagViews<'a> {
    bytes: &'a [u8],
    tag_table: usize,
    index: usize,
    end: usize,
}

impl<'a> Iterator for TagViews<'a> {
    type Item = Result<(&'a str, &'a str), CodecError>;

    /// 借用下一組 tag key/value；descriptor 或 UTF-8 payload 無效時回傳錯誤。
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.end {
            return None;
        }
        let tag = self.tag_table + self.index * TAG_DESCRIPTOR_BYTES;
        self.index += 1;
        Some((|| {
            let key = view_string(
                self.bytes,
                get_u32(self.bytes, tag)?,
                get_u32(self.bytes, tag + 4)?,
            )?;
            let value = view_string(
                self.bytes,
                get_u32(self.bytes, tag + 8)?,
                get_u32(self.bytes, tag + 12)?,
            )?;
            Ok((key, value))
        })())
    }
}

pub struct LogEntryView<'a> {
    pub id: u64,
    pub timestamp: &'a str,
    pub level: LogLevel,
    pub message: &'a str,
    pub tags: TagViews<'a>,
}

pub struct EntryViews<'a> {
    bytes: &'a [u8],
    count: usize,
    tag_count: usize,
    tag_table: usize,
    index: usize,
}

/// 驗證 entries request 的固定 descriptor 區域並建立 zero-copy entry iterator。
pub fn entry_views(bytes: &[u8]) -> Result<EntryViews<'_>, CodecError> {
    let count = get_u32(bytes, 0)?;
    let tag_count = get_u32(bytes, 4)?;
    let tag_table = ENTRY_HEADER_BYTES
        .checked_add(ENTRY_DESCRIPTOR_BYTES * count)
        .ok_or_else(|| CodecError::new("entry descriptor overflow"))?;
    let descriptors_end = tag_table
        .checked_add(TAG_DESCRIPTOR_BYTES * tag_count)
        .ok_or_else(|| CodecError::new("tag descriptor overflow"))?;
    if descriptors_end > bytes.len() {
        return Err(CodecError::new("truncated entry or tag descriptors"));
    }
    Ok(EntryViews {
        bytes,
        count,
        tag_count,
        tag_table,
        index: 0,
    })
}

impl<'a> Iterator for EntryViews<'a> {
    type Item = Result<LogEntryView<'a>, CodecError>;

    /// 借用下一筆 entry 與其 tag iterator，並驗證所有 descriptor 範圍。
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.count {
            return None;
        }
        let descriptor = ENTRY_HEADER_BYTES + self.index * ENTRY_DESCRIPTOR_BYTES;
        self.index += 1;
        Some((|| {
            let tag_index = get_u32(self.bytes, descriptor + 28)?;
            let tag_length = get_u32(self.bytes, descriptor + 32)?;
            let tag_end = tag_index
                .checked_add(tag_length)
                .ok_or_else(|| CodecError::new("tag range overflow"))?;
            if tag_end > self.tag_count {
                return Err(CodecError::new("tag range is out of bounds"));
            }
            Ok(LogEntryView {
                id: get_u64(self.bytes, descriptor)?,
                timestamp: view_string(
                    self.bytes,
                    get_u32(self.bytes, descriptor + 8)?,
                    get_u32(self.bytes, descriptor + 12)?,
                )?,
                level: LogLevel::from_byte(
                    *self
                        .bytes
                        .get(descriptor + 16)
                        .ok_or_else(|| CodecError::new("missing level"))?,
                )?,
                message: view_string(
                    self.bytes,
                    get_u32(self.bytes, descriptor + 20)?,
                    get_u32(self.bytes, descriptor + 24)?,
                )?,
                tags: TagViews {
                    bytes: self.bytes,
                    tag_table: self.tag_table,
                    index: tag_index,
                    end: tag_end,
                },
            })
        })())
    }
}

/// 配置精確大小的 buffer 並編碼 parse request，主要供測試與相容 API 使用。
pub fn encode_parse_request(records: &[String]) -> Vec<u8> {
    let mut out = vec![0; parse_request_size(records)];
    write_parse_request(records, &mut out).expect("correctly sized parse request");
    out
}

/// 將 parse request views 轉成 owned strings，並驗證完整 layout。
pub fn decode_parse_request(bytes: &[u8]) -> Result<Vec<String>, CodecError> {
    record_views(bytes)?
        .map(|record| record.map(str::to_owned))
        .collect()
}

/// 配置精確大小的 buffer 並編碼 entries request。
pub fn encode_entries(entries: &[LogEntry]) -> Vec<u8> {
    let mut out = vec![0; entries_request_size(entries)];
    write_entries_request(entries, &mut out).expect("correctly sized entries request");
    out
}

/// 將 zero-copy entry/tag views 重建為 owned `LogEntry` 集合。
pub fn decode_entries(bytes: &[u8]) -> Result<Vec<LogEntry>, CodecError> {
    entry_views(bytes)?
        .map(|entry| {
            let entry = entry?;
            Ok(LogEntry {
                id: entry.id,
                timestamp: entry.timestamp.to_owned(),
                level: entry.level,
                message: entry.message.to_owned(),
                tags: entry
                    .tags
                    .map(|tag| tag.map(|(key, value)| (key.to_owned(), value.to_owned())))
                    .collect::<Result<_, _>>()?,
            })
        })
        .collect()
}

/// 計算 compact Parse response 的精確位元組長度。
///
/// 編碼前先計算長度，使 guest 只配置一次 response；後續 `push_*` 不需成長或搬移 buffer。
pub fn parsed_entries_encoded_size(entries: &[ParsedEntry]) -> usize {
    let mut size = 4usize; // entry count
    for entry in entries {
        size = size
            .checked_add(4 + entry.timestamp.len())
            .and_then(|size| size.checked_add(1)) // level
            .and_then(|size| size.checked_add(4 + entry.message.len()))
            .and_then(|size| size.checked_add(4)) // tag count
            .and_then(|size| size.checked_add(4 + entry.targettag.len()))
            .expect("parsed-entry binary size overflow");
        for (key, value) in &entry.tags {
            size = size
                .checked_add(4 + key.len())
                .and_then(|size| size.checked_add(4 + value.len()))
                .expect("parsed-entry tag binary size overflow");
        }
    }
    assert!(
        u32::try_from(size).is_ok(),
        "parsed-entry response exceeds u32"
    );
    size
}

/// 以一次精確配置將 parsed entries 編碼成 owned compact binary response。
pub fn encode_parsed_entries(entries: &[ParsedEntry]) -> Vec<u8> {
    let encoded_size = parsed_entries_encoded_size(entries);
    let mut out = Vec::with_capacity(encoded_size);
    push_u32(&mut out, entries.len());
    for entry in entries {
        push_string(&mut out, &entry.timestamp);
        out.push(entry.level.rank());
        push_string(&mut out, &entry.message);
        push_tags(&mut out, &entry.tags);
        push_string(&mut out, &entry.targettag);
    }
    debug_assert_eq!(out.len(), encoded_size);
    debug_assert_eq!(out.capacity(), encoded_size);
    out
}

/// 解碼 compact Parse response，重建 owned parsed entries 並拒絕尾端資料。
pub fn decode_parsed_entries(bytes: &[u8]) -> Result<Vec<ParsedEntry>, CodecError> {
    let mut reader = Reader::new(bytes);
    let count = reader.read_count()?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ParsedEntry {
            timestamp: reader.read_string()?,
            level: LogLevel::from_byte(reader.read_u8()?)?,
            message: reader.read_string()?,
            tags: read_tags(&mut reader)?,
            targettag: reader.read_string()?,
        });
    }
    reader.finish()?;
    Ok(entries)
}

/// 將 filter results 編碼為 count 及逐筆 `u64 id + u8 keep`。
pub fn encode_filter_results(results: &[FilterResult]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + results.len() * 9);
    push_u32(&mut out, results.len());
    for result in results {
        out.extend_from_slice(&result.id.to_le_bytes());
        out.push(u8::from(result.keep));
    }
    out
}

/// 解碼 filter results，並拒絕非 0/1 的 boolean 或尾端多餘資料。
pub fn decode_filter_results(bytes: &[u8]) -> Result<Vec<FilterResult>, CodecError> {
    let mut reader = Reader::new(bytes);
    let count = reader.read_count()?;
    let mut results = Vec::with_capacity(count);
    for _ in 0..count {
        results.push(FilterResult {
            id: reader.read_u64()?,
            keep: match reader.read_u8()? {
                0 => false,
                1 => true,
                _ => return Err(CodecError::new("invalid boolean")),
            },
        });
    }
    reader.finish()?;
    Ok(results)
}

/// 將 JSON 值轉為 tag 字串；字串保留原值，其餘使用 JSON 表示。
pub fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

/// 將 JSON 字串或數字層級映射成 `LogLevel`，未知值預設為 `Info`。
pub fn map_level(value: Option<&Value>) -> LogLevel {
    match value {
        Some(Value::Number(number)) => match number.as_u64().unwrap_or(1) {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Crit,
            5 => LogLevel::Alert,
            6 => LogLevel::Emerg,
            _ => LogLevel::Info,
        },
        Some(Value::String(level)) if level.eq_ignore_ascii_case("debug") => LogLevel::Debug,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("warn") => LogLevel::Warn,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("error") => LogLevel::Error,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("crit") => LogLevel::Crit,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("alert") => LogLevel::Alert,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("emerg") => LogLevel::Emerg,
        _ => LogLevel::Info,
    }
}

/// 收集固定語言 tag、額外根欄位與 `att` 屬性，維持 WCM parse 行為。
pub fn collect_tags(root: &Value) -> Vec<(String, String)> {
    let mut tags = vec![("lang".to_string(), "Rust".to_string())];
    if let Value::Object(fields) = root {
        for (key, value) in fields {
            if !matches!(key.as_str(), "ts" | "level" | "msg" | "att") {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
        if let Some(Value::Object(attributes)) = fields.get("att") {
            for (key, value) in attributes {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
    }
    tags
}

/// 從 owned 原始 logs 執行與 WCM 相同的 JSON parse 邏輯。
pub fn parse_records(raw_data: &[String]) -> Result<Vec<ParsedEntry>, CodecError> {
    let mut entries = Vec::with_capacity(raw_data.len());
    for raw in raw_data {
        let root: Value =
            serde_json::from_str(raw).map_err(|_| CodecError::new("JSON Parse Error"))?;
        entries.push(ParsedEntry {
            timestamp: root
                .get("ts")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            level: map_level(root.get("level")),
            message: root
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            tags: collect_tags(&root),
            targettag: "A".to_string(),
        });
    }
    Ok(entries)
}

/// 從 compact request 建立 record views，並將其解析成 owned entries。
pub fn parse_record_views(bytes: &[u8]) -> Result<Vec<ParsedEntry>, CodecError> {
    parse_views(record_views(bytes)?)
}

/// 直接迭代 borrowed records 執行 JSON parse，避免先重建所有輸入字串。
pub fn parse_views(views: RecordViews<'_>) -> Result<Vec<ParsedEntry>, CodecError> {
    let mut entries = Vec::with_capacity(views.count);
    for raw in views {
        let root: Value =
            serde_json::from_str(raw?).map_err(|_| CodecError::new("JSON Parse Error"))?;
        entries.push(ParsedEntry {
            timestamp: root
                .get("ts")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            level: map_level(root.get("level")),
            message: root
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            tags: collect_tags(&root),
            targettag: "A".to_string(),
        });
    }
    Ok(entries)
}

/// 對 owned entries 套用 Warn 以上保留規則。
pub fn filter_entries(entries: &[LogEntry]) -> Vec<FilterResult> {
    entries
        .iter()
        .map(|entry| FilterResult {
            id: entry.id,
            keep: entry.level.rank() >= LogLevel::Warn.rank(),
        })
        .collect()
}

/// 從 compact request 建立 entry views，並產生 filter results。
pub fn filter_entry_views(bytes: &[u8]) -> Result<Vec<FilterResult>, CodecError> {
    filter_views(entry_views(bytes)?)
}

/// 直接讀取 borrowed entry 的 ID 與層級以產生保留判定。
pub fn filter_views(views: EntryViews<'_>) -> Result<Vec<FilterResult>, CodecError> {
    views
        .map(|entry| {
            let entry = entry?;
            Ok(FilterResult {
                id: entry.id,
                keep: entry.level.rank() >= LogLevel::Warn.rank(),
            })
        })
        .collect()
}

/// 將字串依 JSON 規則跳脫後附加到輸出 buffer。
fn json_escape(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for &byte in value.as_bytes() {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            byte if byte < 0x20 => {
                out.extend_from_slice(format!("\\u{byte:04x}").as_bytes());
            }
            byte => out.push(byte),
        }
    }
    out.push(b'"');
}

/// 將 JSON body 包裝成 4-byte little-endian length-prefixed frame。
fn write_frame(out: &mut Vec<u8>, entry: &[u8]) {
    out.extend_from_slice(&(entry.len() as u32).to_le_bytes());
    out.extend_from_slice(entry);
}

/// 將 owned entries 格式化為與 WCM 相同的 flat JSON frames。
pub fn format_entries(entries: &[LogEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for log in entries {
        let mut entry = Vec::new();
        entry.extend_from_slice(b"{\"id\":");
        entry.extend_from_slice(log.id.to_string().as_bytes());
        entry.extend_from_slice(b",\"timestamp\":");
        json_escape(&mut entry, &log.timestamp);
        entry.extend_from_slice(b",\"level\":\"");
        entry.extend_from_slice(log.level.name().as_bytes());
        entry.extend_from_slice(b"\",\"message\":");
        json_escape(&mut entry, &log.message);
        for (key, value) in &log.tags {
            entry.push(b',');
            json_escape(&mut entry, key);
            entry.push(b':');
            json_escape(&mut entry, value);
        }
        entry.push(b'}');
        write_frame(&mut out, &entry);
    }
    out
}

/// 從 compact request 建立 entry views，並直接格式化 frames。
pub fn format_entry_views(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    format_views(entry_views(bytes)?)
}

/// 迭代 borrowed entries/tags，產生 flat JSON 而不重建 owned 輸入結構。
pub fn format_views(views: EntryViews<'_>) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    for log in views {
        let log = log?;
        let mut entry = Vec::new();
        entry.extend_from_slice(b"{\"id\":");
        entry.extend_from_slice(log.id.to_string().as_bytes());
        entry.extend_from_slice(b",\"timestamp\":");
        json_escape(&mut entry, log.timestamp);
        entry.extend_from_slice(b",\"level\":\"");
        entry.extend_from_slice(log.level.name().as_bytes());
        entry.extend_from_slice(b"\",\"message\":");
        json_escape(&mut entry, log.message);
        for tag in log.tags {
            let (key, value) = tag?;
            entry.push(b',');
            json_escape(&mut entry, key);
            entry.push(b':');
            json_escape(&mut entry, value);
        }
        entry.push(b'}');
        write_frame(&mut out, &entry);
    }
    Ok(out)
}

/// 為 parsed entries 依序配置 ID，轉成 filter/format 使用的 owned entries。
pub fn log_entries_from_parsed(entries: &[ParsedEntry]) -> Vec<LogEntry> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| LogEntry {
            id: index as u64,
            timestamp: entry.timestamp.clone(),
            level: entry.level,
            message: entry.message.clone(),
            tags: entry.tags.clone(),
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "bench")]
unsafe extern "C" {
    /// 呼叫 host 提供的 `bench::time-sign` 以記錄 H1 或 H2。
    #[link_name = "time-sign"]
    fn imported_time_sign();
}

#[cfg(target_arch = "wasm32")]
/// 通知 host 記錄目前 benchmark 時間邊界。
pub fn time_sign() {
    unsafe { imported_time_sign() };
}

/// 將 response buffer 所有權交給 host，並把指標與長度封裝成 `u64`。
pub fn leak_bytes(bytes: Vec<u8>) -> u64 {
    let output = bytes.into_boxed_slice();
    let len = output.len() as u32;
    let ptr = Box::into_raw(output) as *mut u8 as u32;
    (u64::from(ptr) << 32) | u64::from(len)
}

/// 以精確 layout 在 guest 線性記憶體配置未初始化 request buffer。
pub fn alloc_bytes(len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let layout = std::alloc::Layout::array::<u8>(len as usize).expect("valid input layout");
    let pointer = unsafe { std::alloc::alloc(layout) };
    if pointer.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    pointer as u32
}

/// 釋放由 `alloc_bytes` 或相容 response 配置建立的 ABI buffer。
///
/// # Safety
///
/// `ptr` 與 `len` 必須描述此 ABI 先前配置且尚未釋放的 buffer，並只能釋放一次。
pub unsafe fn dealloc_bytes(ptr: u32, len: u32) {
    if ptr != 0 && len != 0 {
        let layout = std::alloc::Layout::array::<u8>(len as usize).expect("valid dealloc layout");
        unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建立 codec round-trip tests 共用的固定 entry。
    fn sample_entry() -> LogEntry {
        LogEntry {
            id: 7,
            timestamp: "2026-08-16T00:00:00Z".to_string(),
            level: LogLevel::Warn,
            message: "hello".to_string(),
            tags: vec![("lang".to_string(), "Rust".to_string())],
        }
    }

    #[test]
    /// 確認 parse request 的 layout 與 owned decode round trip 維持穩定。
    fn parse_request_round_trip_and_layout_are_stable() {
        let encoded = encode_parse_request(&["{}".to_string(), "x".to_string()]);
        assert_eq!(
            encoded,
            vec![
                2, 0, 0, 0, 20, 0, 0, 0, 2, 0, 0, 0, 22, 0, 0, 0, 1, 0, 0, 0, b'{', b'}', b'x'
            ]
        );
        assert_eq!(decode_parse_request(&encoded).unwrap(), ["{}", "x"]);
    }

    #[test]
    /// 確認 entries request 可無損解碼回 owned entries。
    fn entries_round_trip() {
        let entries = vec![sample_entry()];
        assert_eq!(decode_entries(&encode_entries(&entries)).unwrap(), entries);
    }

    #[test]
    /// 確認 parsed response 使用精確配置且可無損 round trip。
    fn parsed_entries_round_trip() {
        let parsed = ParsedEntry {
            timestamp: "t".to_string(),
            level: LogLevel::Info,
            message: "m".to_string(),
            tags: vec![("k".to_string(), "v".to_string())],
            targettag: "A".to_string(),
        };
        let encoded = encode_parsed_entries(std::slice::from_ref(&parsed));
        assert_eq!(
            encoded.len(),
            parsed_entries_encoded_size(std::slice::from_ref(&parsed))
        );
        assert_eq!(encoded.capacity(), encoded.len());
        assert_eq!(decode_parsed_entries(&encoded).unwrap(), [parsed]);
    }

    #[test]
    /// 確認 filter result 的 compact layout 與 decode 維持穩定。
    fn filter_result_layout_is_stable() {
        let results = vec![FilterResult { id: 7, keep: true }];
        assert_eq!(
            encode_filter_results(&results),
            vec![1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(
            decode_filter_results(&encode_filter_results(&results)).unwrap(),
            results
        );
    }

    #[test]
    /// 確認 decoders 拒絕尾端多餘資料及截斷的 descriptors。
    fn decoder_rejects_trailing_and_truncated_data() {
        assert!(decode_parse_request(&[0, 0, 0, 0, 1]).is_err());
        assert!(decode_entries(&[1, 0, 0, 0]).is_err());
    }
}
