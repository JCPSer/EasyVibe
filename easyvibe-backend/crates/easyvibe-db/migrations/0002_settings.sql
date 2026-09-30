-- 域 2：配置体系（backend-design §10；敏感值加密 at rest，明文不落盘）
CREATE TABLE IF NOT EXISTS settings (
  scope TEXT NOT NULL,              -- 'global' 或 repo_id
  key TEXT NOT NULL,                -- 如 llm.service.default / llm.service.default.apiKey / slot.patrol
  value TEXT NOT NULL,              -- JSON（敏感 key 的值为加密信封 JSON）
  encrypted INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);
