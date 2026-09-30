//! 基础层：错误类型、事件名常量、ID/时间工具。无任何内部依赖。
use serde::Serialize;

pub mod events {
    /// WS 事件名（两级 camelCase，见 backend-design.md §4）
    pub const MAP_CHANGED: &str = "map.changed";
    pub const MAP_INVALID: &str = "map.invalid";
    pub const GROWTH_EVENT: &str = "growth.event";
    pub const PROGRESS_UPDATED: &str = "progress.updated";
    pub const SESSION_STATUS_CHANGED: &str = "session.statusChanged";
    pub const AGENT_SLOT_UPDATED: &str = "agent.slotUpdated";
}

/// 统一 API 错误（状态码映射见 backend-design.md §5）
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("map invalid: {0}")]
    MapInvalid(String),
    #[error("internal: {0}")]
    Internal(String),
}

/// 统一成功响应包
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), message: None }
    }
}

/// 统一错误响应包
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
    pub code: String,
}

// ---------- 敏感配置加密（backend-design §10 #5：API key 加密 at rest） ----------

use aes_gcm::aead::{Aead, KeyInit as _};
use aes_gcm::{Aes256Gcm, Nonce};

/// 本地主密钥加密器（Aes256Gcm，随机 nonce，信封格式 {v:1, n: base64, d: base64}）。
/// 主密钥来源：EASYVIBE_MASTER_KEY（64 位十六进制）或数据目录 .master_key 文件（0600，首次生成）。
pub struct SecretCipher {
    cipher: Aes256Gcm,
}

/// Y4 清债：密钥源抽象——主密钥换源（0600 文件 → 系统钥匙串 → KMS）时
/// 只换 Provider 实现，调用方零改动（技术审查伏笔第 1 项）
pub trait KeyProvider: Send + Sync {
    /// 返回 32 字节主密钥
    fn key(&self) -> Result<[u8; 32], ApiError>;
}

/// 文件密钥源（一期定稿）：EASYVIBE_MASTER_KEY（64 位十六进制）优先，
/// 否则数据目录 .master_key（0600，首次生成）
pub struct FileKeyProvider {
    pub env_var: Option<String>,
    pub file_path: String,
}

impl FileKeyProvider {
    pub fn new(data_dir: &str) -> Self {
        Self {
            env_var: std::env::var("EASYVIBE_MASTER_KEY").ok(),
            file_path: format!("{data_dir}/.master_key"),
        }
    }
}

impl KeyProvider for FileKeyProvider {
    fn key(&self) -> Result<[u8; 32], ApiError> {
        if let Some(hex) = &self.env_var {
            let bytes = (0..hex.len().saturating_sub(1))
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| ApiError::Internal(format!("主密钥不是合法十六进制: {e}")))?;
            if bytes.len() == 32 {
                return Ok(bytes.try_into().map_err(|_| ApiError::Internal("主密钥长度异常".into()))?);
            }
        }
        let hex = std::fs::read_to_string(&self.file_path).unwrap_or_else(|_| {
            let mut bytes = [0u8; 32];
            getrandom::getrandom(&mut bytes).expect("主密钥生成失败");
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            std::fs::write(&self.file_path, &hex).expect("写主密钥文件失败");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let _ = std::fs::set_permissions(&self.file_path, std::fs::Permissions::from_mode(0o600));
            }
            hex
        });
        let hex = hex.trim();
        let bytes = (0..hex.len())
            .step_by(2)
            .take(32)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|e| ApiError::Internal(format!("主密钥文件损坏: {e}")))?;
        bytes.try_into().map_err(|_| ApiError::Internal("主密钥文件长度异常".into()))
    }
}

impl SecretCipher {
    /// Y4：从密钥源构造——二期换钥匙串只需新增 KeyProvider 实现
    pub fn from_provider(provider: &dyn KeyProvider) -> Result<Self, ApiError> {
        Self::from_bytes(&provider.key()?)
    }
}

impl SecretCipher {
    pub fn from_hex_key(hex: &str) -> Result<Self, ApiError> {
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|e| ApiError::Internal(format!("主密钥不是合法十六进制: {e}")))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(key: &[u8]) -> Result<Self, ApiError> {
        if key.len() != 32 {
            return Err(ApiError::Internal(format!("主密钥必须是 32 字节，实际 {}", key.len())));
        }
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| ApiError::Internal(format!("加密器初始化失败: {e}")))?;
        Ok(Self { cipher })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, ApiError> {
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| ApiError::Internal(format!("nonce 生成失败: {e}")))?;
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|e| ApiError::Internal(format!("加密失败: {e}")))?;
        Ok(serde_json::json!({
            "v": 1,
            "n": base64_encode(&nonce_bytes),
            "d": base64_encode(&ct),
        })
        .to_string())
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String, ApiError> {
        let v: serde_json::Value = serde_json::from_str(envelope).map_err(|_| ApiError::Internal("加密信封不是合法 JSON".into()))?;
        let nonce = base64_decode(v["n"].as_str().unwrap_or(""))?;
        let data = base64_decode(v["d"].as_str().unwrap_or(""))?;
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce), data.as_slice())
            .map_err(|_| ApiError::Internal("解密失败（主密钥不匹配或数据损坏）".into()))?;
        String::from_utf8(pt).map_err(|_| ApiError::Internal("解密结果不是 UTF-8".into()))
    }
}

// 无依赖 base64（配置体量小，不引 base64 crate）
fn base64_encode(b: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((b.len() + 2) / 3 * 4);
    for chunk in b.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ApiError> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= match c {
                b'=' => 0,
                _ => val(c).ok_or_else(|| ApiError::Internal("base64 含非法字符".into()))? << (18 - 6 * i),
            };
        }
        out.push((n >> 16) as u8);
        if pad < 2 { out.push((n >> 8) as u8); }
        if pad < 1 { out.push(n as u8); }
    }
    Ok(out)
}

#[cfg(test)]
mod crypto_tests {
    use super::*;

    #[test]
    fn key_provider_file_source_roundtrip() {
        // Y4 回归：文件密钥源首次生成 → 二次读取一致 → from_provider 构造的加解密互通
        let dir = std::env::temp_dir().join(format!("ev-keyprovider-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let provider = crate::FileKeyProvider::new(&dir.to_string_lossy());
        let k1 = provider.key().unwrap();
        let k2 = provider.key().unwrap();
        assert_eq!(k1, k2, "二次读取必须一致");
        let c1 = crate::SecretCipher::from_provider(&provider).unwrap();
        let hex: String = k1.iter().map(|x| format!("{x:02x}")).collect();
        let c2 = crate::SecretCipher::from_hex_key(&hex).unwrap();
        let sealed = c1.encrypt("hello").unwrap();
        assert_eq!(c2.decrypt(&sealed).unwrap(), "hello", "provider 构造与直接构造必须互通");
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let cipher = SecretCipher::from_hex_key(&"ab".repeat(32)).unwrap();
        let secret = "sk-test-123-中文也可以";
        let env = cipher.encrypt(secret).unwrap();
        assert!(!env.contains("sk-test"), "密文不得包含明文");
        assert_eq!(cipher.decrypt(&env).unwrap(), secret);
    }

    #[test]
    fn wrong_key_fails() {
        let c1 = SecretCipher::from_hex_key(&"ab".repeat(32)).unwrap();
        let c2 = SecretCipher::from_hex_key(&"cd".repeat(32)).unwrap();
        let env = c1.encrypt("x").unwrap();
        assert!(c2.decrypt(&env).is_err());
    }
}
