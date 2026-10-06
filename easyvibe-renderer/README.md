# React + TypeScript + Vite

This template provides a minimal setup to get React working in Vite with HMR and some ESLint rules.

Currently, two official plugins are available:

- [@vitejs/plugin-react](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react) uses [Babel](https://babeljs.io/) (or [oxc](https://oxc.rs) when used in [rolldown-vite](https://vite.dev/guide/rolldown)) for Fast Refresh
- [@vitejs/plugin-react-swc](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react-swc) uses [SWC](https://swc.rs/) for Fast Refresh

## React Compiler

The React Compiler is not enabled on this template because of its impact on dev & build performances. To add it, see [this documentation](https://react.dev/learn/react-compiler/installation).

## Expanding the ESLint configuration

If you are developing a production application, we recommend updating the configuration to enable type-aware lint rules:

```js
export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      // Other configs...

      // Remove tseslint.configs.recommended and replace with this
      tseslint.configs.recommendedTypeChecked,
      // Alternatively, use this for stricter rules
      tseslint.configs.strictTypeChecked,
      // Optionally, add this for stylistic rules
      tseslint.configs.stylisticTypeChecked,

      // Other configs...
    ],
    languageOptions: {
      parserOptions: {
        project: ['./tsconfig.node.json', './tsconfig.app.json'],
        tsconfigRootDir: import.meta.dirname,
      },
      // other options...
    },
  },
])
```

You can also install [eslint-plugin-react-x](https://github.com/Rel1cx/eslint-react/tree/main/packages/plugins/eslint-plugin-react-x) and [eslint-plugin-react-dom](https://github.com/Rel1cx/eslint-react/tree/main/packages/plugins/eslint-plugin-react-dom) for React-specific lint rules:

```js
// eslint.config.js
import reactX from 'eslint-plugin-react-x'
import reactDom from 'eslint-plugin-react-dom'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      // Other configs...
      // Enable lint rules for React
      reactX.configs['recommended-typescript'],
      // Enable lint rules for React DOM
      reactDom.configs.recommended,
    ],
    languageOptions: {
      parserOptions: {
        project: ['./tsconfig.node.json', './tsconfig.app.json'],
        tsconfigRootDir: import.meta.dirname,
      },
      // other options...
    },
  },
])
```

## 组件归属规则（c-arch-3）

> 规则权威落点：`src/__tests__/archGuard.test.ts` 文件头注释 + 断言组 7；
> CI 可见等价判据：`scripts/check_components_ownership.py`。本 README 只写判定序，不复制集合/数字（唯一事实源在守卫常量）。

判定序（按序，首个命中即定）：

| # | 判据 | 落点 |
|---|---|---|
| ① | 路由可达整页（`PageId` 装配表引用） | `src/pages/` |
| ② | 消费图数据 / ReactFlow 的画布件 | `src/components/canvas/` |
| ③ | 浮层内容、全局浮标（无路由） | `src/components/overlays/` |
| ④ | 装配壳 chrome（页头 / 窗控 / 主题 / 装配壳） | `src/components/shell/` |
| ⑤ | 其余可复用业务组件 | `src/components/<既有域>/` |
| ⑥ | **禁止**：`src/components/` 根目录存在任何 `*.tsx`（及任何文件） | — |

配套命名约定（非强制）：`*Page.tsx` 仅用于 ①；`*Panel` / `*List` / `*Form` 用于 ②/③/⑤。

变更流程：新增组件若不满足 ①–⑤ 任一条 → 先提规则修订（改守卫 + 改规则），再落文件。
「先落文件，再看测试红不红」是本规则明确禁止的路径。

守卫对应关系：

| 规则 | 守卫 |
|---|---|
| ① `pages/` | `componentGuard` 组 1（`src/pages/*.tsx` 全等）；`archGuard` 组 3（页面不得反向 import App/routes） |
| ② `canvas/` | `archGuard` 组 2（`MAP_CANVAS_FILES = walk(canvas)`）；`componentGuard` 组 4（`canvas/**` 全等） |
| ③④ `overlays/` `shell/` | `componentGuard` 组 4（两目录全等）；`repoLayout` 组 1（≤300） |
| ⑤ 域目录 | 既有 chat/settings/taskworkflow 三域守卫 |
| ⑥ 根目录禁 `*.tsx` | `archGuard` 组 7 ＋ `scripts/check_components_ownership.py` C1/C2（CI） |
