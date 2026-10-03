# EasyVibe

**The VibeCoding IDE with built-in architecture governance.**
*Humans own structure and boundaries; agents own implementation and details.*

[中文说明](README.zh-CN.md)

![UI concept](主界面示意图渲染.png)

> As AI code generation gets cheaper, governance becomes more valuable.
> As vibe coding spreads, repos rot faster — EasyVibe exists to keep humans in control.

## What is EasyVibe?

In the vibe-coding era, a wall sits between the human and the codebase: the agent writes the code, and the human no longer knows what the repo looks like, where module boundaries are, or how severe the coupling has become. Traditional IDEs are designed for *humans writing code* — file trees and text editors. The agent era needs an **architecture view**: modules, responsibilities, dependencies, and blast radius.

EasyVibe is a local-first, cross-platform IDE for **existing codebases** that combines three things in one product:

| Pillar | What it does |
|---|---|
| **Semantic Code Map** | LLM-generated architecture map stored *inside the repo* (`.easyvibe/map/`), rendered as a dynamic layered diagram — layer count, names and responsibilities are all derived from the actual project, not hardcoded. |
| **Health Audit** | Module-level **and** architecture-level health assessment (every module healthy ≠ the architecture is healthy). Patrol runs inject the previous baseline as context; git change tracking ranks how stale each module has become. |
| **Harness-governed Agent Workflows** | Every development task flows through the harness: requirements matrix → solution design → implementation → code review, with gates, approvals, and full in-repo traceability. Guardrails are injected, never hard-blocked. |

## Feature Highlights

- **Architecture map** — dynamic layered canvas (React Flow), module health rings, reverse-dependency edges, concern prioritization, filtered views (violations only / issues only / dependencies only)
- **Module drill-down** — expand a module into a lazy-loaded depth-1 submap (internal wiring diagram), cached and remapped when the parent module evolves
- **Custom views** — turn any conversation answer into a persistent, reusable view (`.easyvibe/views/`), stored as entity references + annotations, never layouts
- **Agent providers** — plug in Claude Code / Codex / OpenCode via presets, or any Anthropic/OpenAI-compatible endpoint; bind different LLM services to different duty slots (cheap model for map patrol, strong model for architecture analysis); built-in connection testing
- **Agent chat** — streaming terminal with real-time `stream-json` output, interrupt, markdown & mermaid rendering, image attachments
- **Task orchestration** — kanban + pipeline views over the harness workflow; manual (plan → diff → review gates) or auto approval, everything traceable
- **Git integration** — worktree support, change tracking, history
- **Multi-repo workspace** — register and switch between local repositories in-app
- **Zero-config standalone** — a single backend `.exe` with the UI embedded; double-click and it opens in your browser (Windows & macOS)
- **Desktop app** — Tauri v2 shell for macOS and Windows (Windows installer built via GitHub Actions)

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  easyvibe-desktop (Tauri v2 shell, macOS/Windows)   │
│  easyvibe-renderer (React 18 + Vite + React Flow)   │
├─────────────────────────────────────────────────────┤
│  easyvibe-backend (Rust workspace)                  │
│    easyvibe-app      REST + WS assembly, task exec  │
│    easyvibe-session  CLI agent sessions (write-path)│
│    easyvibe-ai-agent LLM slots (patrol / QA)        │
│    easyvibe-map      map service & file watchers    │
│    easyvibe-db       SQLite (IDE state)             │
│    easyvibe-common   crypto, errors, events         │
│    easyvibe-api-types shared contracts              │
└─────────────────────────────────────────────────────┘
```

**Two-domain storage model** — repo-derived data (map, submaps, views, development docs) lives as JSON files inside the repo: git-visible, agent-writable, re-inducible. IDE-owned state (conversations, task/approval records, health history) lives in SQLite. The map never moves into a database.

## Quick Start

### Desktop app (recommended)

```bash
./scripts/dev.sh /path/to/your/repo
```

Starts the backend (7101) and the renderer dev server (7100), opens the map of your repo.

### Standalone backend (zero config)

Download the single-file backend binary (see Releases), double-click it — the UI opens in your browser. Prompts, schema, and frontend are all embedded; nothing else is needed. Or build it yourself:

```bash
cd easyvibe-backend
cargo build --release -p easyvibe-app
./target/release/easyvibe-backend
```

### From source

```bash
# Backend
cd easyvibe-backend && cargo build

# Frontend
cd easyvibe-renderer && npm install && npm run dev
```

Cross-compile the Windows backend from macOS (llvm-mingw, statically linked, no DLL dependencies):

```bash
export PATH=<llvm-mingw>/bin:$PATH
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-pc-windows-gnullvm -p easyvibe-app
```

## Status

**Pre-alpha, in active development.** Interfaces, data formats, and UX are all evolving. macOS and Windows are the primary targets. Feedback and contributions are welcome.

## Repository Layout

| Path | Description |
|---|---|
| `easyvibe-backend/` | Rust workspace — REST/WS server, agent sessions, LLM slots, SQLite state |
| `easyvibe-renderer/` | React + Vite frontend (the architecture map UI) |
| `easyvibe-desktop/` | Tauri v2 desktop shell + Windows CI packaging |
| `easyvibe-*.md` / `easyvibe-map-schema-v1.json` | Map-generation prompts and the map data format schema |
| `EasyVibe产品需求文档-v0.3.md` | Product requirements (PRD, Chinese) |
| `EasyVibe地图数据格式规范-v1.0.md` | Map data format spec (Chinese) |
| `docs/` | Design briefs, requirements matrices, solution designs, audits |
| `ui-mockups/` | UI prototypes |
| `scripts/` | Dev launcher, live-fire WS test, desktop sync |
| `.github/workflows/` | Windows installer CI (triggered on `v*` tags) |

## License

License pending — to be announced before the first public release.
