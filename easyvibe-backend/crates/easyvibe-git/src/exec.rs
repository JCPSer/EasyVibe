//! git 命令执行封装：异步写/读统一超时；同步探针用于非致命读取。

use easyvibe_common::ApiError;
use std::path::Path;
use std::time::Duration;

pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn git(repo: &Path, args: &[&str]) -> Result<String, ApiError> {
    let out = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git").args(args).current_dir(repo).output(),
    )
    .await
    .map_err(|_| ApiError::Internal(format!("git {args:?} 超时（{GIT_TIMEOUT:?}）")))?
    .map_err(|e| ApiError::Internal(format!("git 执行失败: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::BadRequest(format!("git {args:?} 失败: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn git_opt(repo: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(repo).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
