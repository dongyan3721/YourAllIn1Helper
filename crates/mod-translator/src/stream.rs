//! SSE (Server-Sent Events) 与多字节跨块 UTF-8 边界安全流式解析器。
//!
//! 关键技术点：
//! 1. UTF-8 边界缓冲：中日文字符（多为 3 字节）或 Emoji（4 字节）在网络 TCP/SSE 分片切断时可能落在多块边界上。
//!    `Utf8ChunkBuffer` 会保留未完成的尾部字节，与下一分片拼接后再转为有效 Rust String，绝不 panic 或丢字。
//! 2. SSE 行解析：支持 `data: ...` 行拼接、空行分段与 `[DONE]` 终止符。

use serde_json::Value;

/// UTF-8 跨块安全收集缓冲器。
#[derive(Debug, Default)]
pub struct Utf8ChunkBuffer {
    leftover: Vec<u8>,
}

impl Utf8ChunkBuffer {
    pub fn new() -> Self {
        Self {
            leftover: Vec::new(),
        }
    }

    /// 接收一段原始二进制分片，输出当前可以确认的合法 UTF-8 字符串，并将未完结的末尾字节保留至下次拼接。
    pub fn push_bytes(&mut self, chunk: &[u8]) -> String {
        let mut buffer = Vec::with_capacity(self.leftover.len() + chunk.len());
        buffer.extend_from_slice(&self.leftover);
        buffer.extend_from_slice(chunk);
        self.leftover.clear();

        if buffer.is_empty() {
            return String::new();
        }

        // 从后往前寻找最近的有效 UTF-8 边界
        let mut valid_up_to = buffer.len();
        loop {
            match std::str::from_utf8(&buffer[..valid_up_to]) {
                Ok(_) => break,
                Err(e) => {
                    if e.valid_up_to() > 0 {
                        valid_up_to = e.valid_up_to();
                        break;
                    } else if valid_up_to > 0 {
                        // 尝试缩减 1 字节检查是否为不完整字符
                        valid_up_to -= 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // 保存切断的未完成字节
        if valid_up_to < buffer.len() {
            // 最多留存 4 字节（UTF-8 最长单字符 4 字节）
            let unparsed = &buffer[valid_up_to..];
            if unparsed.len() <= 4 {
                self.leftover.extend_from_slice(unparsed);
            }
        }

        if valid_up_to > 0 {
            String::from_utf8_lossy(&buffer[..valid_up_to]).to_string()
        } else {
            String::new()
        }
    }

    /// 结束流时刷写剩余的全部字节（采用 lossy 容错处理）。
    pub fn finish(&mut self) -> String {
        if self.leftover.is_empty() {
            String::new()
        } else {
            let s = String::from_utf8_lossy(&self.leftover).to_string();
            self.leftover.clear();
            s
        }
    }
}

/// 解析 OpenAI 兼容接口的 SSE 单行 `data: {...}` 内容。
pub fn parse_sse_openai_delta(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("data:") {
        return None;
    }
    let data_str = trimmed.trim_start_matches("data:").trim();
    if data_str == "[DONE]" {
        return None;
    }
    let v: Value = serde_json::from_str(data_str).ok()?;
    // choices[0].delta.content
    let content = v
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()?;
    Some(content.to_string())
}
