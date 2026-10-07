#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""三分守卫共享的**手写**合成网格构件（c-arch-12 / R8）。

背景：check_console_granularity / check_renderer_granularity / check_host_boundary 的
`--selfcheck` 各带一份手写合成图，其中渲染器四格 / settings 格 / 装配壳的 glob 字面量
逐字重复。本文件把这些 glob 字面量收敛为**一份**（守卫内嵌 glob 副本 3 文件 → 1 文件）。

纪律（R8 / Q5 甲案）：合成图是判据的**自证载体**，必须**手写**，**不得**由 policy 派生——
否则判据与被测共享同一错误源（policy 错 → 判据同错 → 同错假绿）。守卫另在 `--check` 真图入口
加「探针归属 ⊆ policy」断言，把「合成缩图」与「policy 规范网格」在**探针级**对齐，
既去重又不牺牲 selfcheck 的独立性。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""

# ---- 渲染器四格（renderer 守卫口径）
RENDERER_RUNTIME = ["easyvibe-renderer/src/runtime/**"]
RENDERER_API = ["easyvibe-renderer/src/api/**"]
RENDERER_SHARED = ["easyvibe-renderer/src/shared/**", "easyvibe-renderer/src/types/**"]
UI_KIT = ["easyvibe-renderer/src/components/ui/**",
          "easyvibe-renderer/src/lib/utils.ts",
          "easyvibe-renderer/src/hooks/use-mobile.ts"]

# ---- console 侧 settings 格、lib 三类能力格与装配壳
SETTINGS = ["easyvibe-renderer/src/components/settings/**"]
# c-arch-14：lib/** 三类非装配能力外提为独立呈现格，各恰 1 条**精确文件 glob**（无通配）。
# 合成图必须**手写**（不得由 policy 派生），否则判据与被测共享同一错误源。
HEALTH_REPORT = ["easyvibe-renderer/src/lib/healthReport.ts"]
ONBOARDING = ["easyvibe-renderer/src/lib/onboarding.ts"]
UPDATER = ["easyvibe-renderer/src/lib/updater.ts"]
CONSOLE_ASSEMBLY = ["easyvibe-renderer/src/pages/**",
                    "easyvibe-renderer/src/components/shell/**",
                    "easyvibe-renderer/src/components/overlays/**"]
CONSOLE_APP = ["easyvibe-renderer/src/App.tsx", "easyvibe-renderer/src/pages/**",
               "easyvibe-renderer/src/lib/**", "easyvibe-renderer/src/hooks/**"]

# ---- 工具链格（两种缩图口径）
MAP_TOOLCHAIN_CONSOLE = ["scripts/**"]
MAP_TOOLCHAIN_RENDERER = ["run/**", "scripts/**", "easyvibe-renderer/scripts/**"]

# ---- app-entry 宿主适配格
INDEX_HTML = "easyvibe-renderer/index.html"
HOST_ADAPTER_PREFIX = "easyvibe-renderer/src/host-adapter/"


def console_green_map():
    """console 守卫合成「拆分后」绿图：settings-ui + lib 三类能力格 + console-ui（仅装配壳）。

    ★ console-ui 的 files 不含 `src/lib/**`（c-arch-14 已外提）；ui-kit 用完整三 glob
    （含 `src/lib/utils.ts`），故 utils.ts 在合成图里唯一归 ui-kit（ΔS5）。
    """
    return {"modules": [
        {"id": "map-toolchain", "files": list(MAP_TOOLCHAIN_CONSOLE)},
        {"id": "renderer-runtime", "files": list(RENDERER_RUNTIME)},
        {"id": "renderer-api", "files": list(RENDERER_API)},
        {"id": "ui-kit", "files": list(UI_KIT)},
        {"id": "health-report", "files": list(HEALTH_REPORT)},
        {"id": "onboarding-state", "files": list(ONBOARDING)},
        {"id": "app-updater", "files": list(UPDATER)},
        {"id": "console-ui", "files": list(CONSOLE_ASSEMBLY)},
        {"id": "settings-ui", "files": list(SETTINGS)},
    ]}


def renderer_green_map():
    """renderer 守卫合成「拆分后」四格绿图：四格 + map-toolchain + console-ui。"""
    return {"modules": [
        {"id": "map-toolchain", "files": list(MAP_TOOLCHAIN_RENDERER)},
        {"id": "renderer-runtime", "files": list(RENDERER_RUNTIME)},
        {"id": "renderer-api", "files": list(RENDERER_API)},
        {"id": "renderer-shared", "files": list(RENDERER_SHARED)},
        {"id": "ui-kit", "files": list(UI_KIT)},
        {"id": "console-ui", "files": list(CONSOLE_APP)},
    ]}
