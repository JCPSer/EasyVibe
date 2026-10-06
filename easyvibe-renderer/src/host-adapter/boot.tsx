// 引导入口（bootstrap）——host-adapter（app-entry / order 0）。
//
// c-arch-5 关键：**谁加载适配器决定依赖方向**。
//   · index.html 的 `<script type="module">` 入口指向本文件（index.html 同归 host-adapter），
//     使「加载适配器」成为 host-adapter(0) 的**模块内**关系，而非 presentation(1) → app-entry(0) 的逆边；
//   · 本文件先静态 import './register'（适配器自注册进端口），再静态 import '@/main'（装载 App），
//     ES 模块静态依赖按声明顺序求值 → 注册必先于 App 挂载，无需 ready-promise、无竞态；
//   · 依赖方向：host-adapter(0) → console-ui(1)（下行可见边）、host-adapter(0) → renderer-runtime(2)（下行）。
//   · src/main.tsx（console-ui）**不** import 本模块或适配器，消费方只经端口 @/runtime/host。
import './register'
import '@/main'
