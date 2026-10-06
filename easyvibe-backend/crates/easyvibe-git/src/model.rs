//! git 数据形状（纯类型，无 IO）。

/// 序列化形状即 HTTP `data` 形状（routes 侧零内联 json!：直接 ApiResponse::ok(to_value(..))）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GitFile {
    /// 归一化状态：M 修改 / A 新增（含暂存）/ D 删除 / R 重命名 / ? 未跟踪
    pub status: char,
    pub path: String,
    /// 重命名来源路径（仅 R）
    pub orig: Option<String>,
    /// 增删行数（未跟踪文件无 diff 数据，为 None）
    pub adds: Option<i64>,
    pub dels: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GitStatus {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub files: Vec<GitFile>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct GitLogRow {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub subject: String,
    /// 本提交触及的文件（--name-only；前端映射模块 chips 用）
    pub files: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFileStat {
    pub path: String,
    pub adds: i64,
    pub dels: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<CommitFileStat>,
}
