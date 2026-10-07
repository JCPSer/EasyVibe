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

/// 单文件统一差异（Git 页 diff 抽屉的数据形状；text 为统一 diff 文本或整文件内容）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// 未跟踪新文件：text 为整文件内容（前端按全量新增渲染）
    pub untracked: bool,
    /// 二进制文件无法文本化展示
    pub binary: bool,
    /// 请求的范围内没有任何差异（如仅暂存的文件请求 unstaged diff）
    pub empty: bool,
    /// 差异超过 DIFF_MAX_LINES，text 已截断
    pub truncated: bool,
    /// 截断前完整差异行数（未截断 = text 行数）
    pub total_lines: i64,
    pub text: String,
}
