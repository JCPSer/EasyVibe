# EasyVibe

**自带架构治理的 VibeCoding IDE。**
*人管结构与边界，Agent 管实现与细节。*

[English README](README.md)

![界面概念图](主界面示意图渲染.png)

> AI 生成越便宜，治理越值钱；vibe coding 越普及，仓库腐化越快，EasyVibe 越必要。

## EasyVibe 是什么？

Vibe coding 时代，人与代码之间隔着 Agent：代码是"长出来"的，人不知道仓库现在长什么样、模块边界在哪、耦合多严重；Agent 一次任务动了哪些文件、是否破坏既有架构，也缺乏直观呈现。传统 IDE 的文件树/文本编辑器视角是为"人写代码"设计的，Agent 时代需要的是**架构视角**：模块、职责、依赖、变更影响面。

EasyVibe 是一款面向**既有代码仓库**的本地优先、跨平台 IDE，把三件事做进一个产品：

| 支柱 | 能力 |
|---|---|
| **语义代码地图** | LLM 归纳的架构地图，存储在**仓库内**（`.easyvibe/map/`），渲染为动态分层架构图——层数、层名、层职责全部由项目实际结构推导，不硬编码。 |
| **架构健康审计** | 模块级 **+ 架构级** 两级健康评估（每个模块都健康 ≠ 架构健康）；巡检时注入上轮健康基线作为上下文；结合 git 变更计算模块落后程度并排序提醒。 |
| **Harness 护栏工作流** | 每个开发任务走完整 harness 流程：需求矩阵 → 方案设计 → 代码开发 → 代码审查，带门禁、审批与全量留痕；护栏靠注入上下文实现，不做硬拦截。 |

## 功能亮点

- **架构地图**——动态分层画布（React Flow），模块健康环、逆向依赖红虚线、问题清单按严重度排序、只看违规/问题/依赖等过滤视图
- **模块展开**——点击模块懒加载深度 1 层子图（内部连线图），父模块演进时自动重映射或提醒作废
- **自定义视图**——对话回答一键「存为视图」（`.easyvibe/views/`），只存实体引用+批注、不存布局，是可复用的数据资产
- **Agent 接入**——Claude Code / Codex / OpenCode 预设，或任意 Anthropic/OpenAI 兼容端点；不同职责槽位绑定不同 LLM 服务（巡检用便宜模型、架构分析用强模型）；内置测试连接
- **对话**——stream-json 真实时终端、可打断、Markdown/Mermaid 渲染、图片附件
- **任务编排**——看板 + 流水线双视图覆盖 harness 全流程；手动（计划→Diff→审查三道关）/自动两档审批，全程留痕可回溯
- **Git 集成**——工作树支持、变更追踪、历史记录
- **多仓库工作区**——应用内注册/切换本地仓库
- **零配置独立形态**——单个后端可执行文件内嵌 UI，双击即出界面（Windows/macOS）
- **桌面应用**——Tauri v2 壳（macOS/Windows），Windows 安装包由 GitHub Actions 构建

## 技术架构

```
┌─────────────────────────────────────────────────────┐
│  easyvibe-desktop（Tauri v2 桌面壳，macOS/Windows）  │
│  easyvibe-renderer（React 18 + Vite + React Flow）   │
├─────────────────────────────────────────────────────┤
│  easyvibe-backend（Rust workspace）                   │
│    easyvibe-app      REST + WS 组装层、任务执行        │
│    easyvibe-session  CLI agent 会话（写路径归纳）      │
│    easyvibe-ai-agent LLM 槽位（巡检/问答）             │
│    easyvibe-map      地图服务与文件监听                │
│    easyvibe-db       SQLite（IDE 自有状态）            │
│    easyvibe-common   加密、错误、事件                  │
│    easyvibe-api-types 共享契约                        │
└─────────────────────────────────────────────────────┘
```

**两域分离存储**——仓库派生数据（地图/子图/视图/开发留痕）是仓库内 JSON 文件：git 可见、agent 可写、可重新归纳；IDE 自有状态（对话历史、任务/审批留痕、健康分历史）存 SQLite。地图永不入库型数据库。

## 快速开始

### 桌面应用（推荐）

```bash
./scripts/dev.sh /path/to/your/repo
```

启动后端（7101）+ 渲染器（7100），打开你仓库的架构地图。

### 独立后端（零配置）

下载单文件后端可执行文件（见 Releases），双击即自动打开浏览器界面。提示词、schema、前端全部内嵌，无需任何配置。也可以自己编译：

```bash
cd easyvibe-backend
cargo build --release -p easyvibe-app
./target/release/easyvibe-backend
```

### 源码构建

```bash
# 后端
cd easyvibe-backend && cargo build

# 前端
cd easyvibe-renderer && npm install && npm run dev
```

macOS 交叉编译 Windows 后端（llvm-mingw，静态链接、零 DLL 依赖）：

```bash
export PATH=<llvm-mingw>/bin:$PATH
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-pc-windows-gnullvm -p easyvibe-app
```

## 项目状态

**Pre-alpha，活跃开发中。** 接口、数据格式、交互都在快速演进，macOS 与 Windows 是主要目标平台。欢迎提 issue 和 PR。

## 仓库结构

| 路径 | 说明 |
|---|---|
| `easyvibe-backend/` | Rust workspace——REST/WS 服务、agent 会话、LLM 槽位、SQLite 状态库 |
| `easyvibe-renderer/` | React + Vite 前端（架构地图 UI） |
| `easyvibe-desktop/` | Tauri v2 桌面壳 + Windows CI 打包 |
| `easyvibe-*.md` / `easyvibe-map-schema-v1.json` | 地图归纳提示词与地图数据格式 schema |
| `EasyVibe产品需求文档-v0.3.md` | 产品需求文档（PRD） |
| `EasyVibe地图数据格式规范-v1.0.md` | 地图数据格式规范 |
| `docs/` | 设计简报、需求矩阵、方案设计、审计报告 |
| `ui-mockups/` | UI 原型图 |
| `scripts/` | 开发启动器、WS 实弹测试、桌面端同步 |
| `.github/workflows/` | Windows 安装包 CI（`v*` 标签触发） |

## 许可证

许可证待定——首个公开发布前会确定并补充。
