# 公共提示词与机制调研维护

[提示词组装](<mod.rs>)提供人格/记忆共用的稳定段拼装，[测试](<tests.rs>)守组装语义；人格档案见[soul](<../soul/AGENTS.md>)，记忆注入见[memory](<../memory/AGENTS.md>)。本目录调研记录中的“未找到证据”事项不能当作既有事实引用。

---

## DSH 提示词与插件机制调研（历史记录）

> **历史设计 / 实施记录**：由原《人格与图记忆》长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/soul-and-graph-memory-plan.md>)定位；当前行为先读本文件上半部分、源码与测试。

## 11. 附录 A：DSH 插件机制二轮调研 —— 事实更正 + 可移植清单

本节补充并**更正** §6，来源是对 asar 内 DSH 源码的第二轮直读（取证方式见文首说明）。

### 11.1 三处事实更正

| 早期说法 | 实测结论 | 影响 |
|---|---|---|
| graph-memory 的 `configSchema` 是「DSH 能在 GUI 自动渲染表单」的依据 | ❌ 那是 **OpenClaw** 侧清单字段；DSH 侧检索 `configSchema` **0 命中**（`contextEngine` 同样 0 命中） | §2.3/§6 的"自动表单"论据撤销（已就地更正）。`GET /api/config/schema` 仍值得做，但定位是**校验 / 默认值 / 字段元数据**，不是"自动生成 UI"的承诺 |
| DSH 有 `contextEngine` 槽位，graph-memory 用的就是它 | ❌ DSH 无此槽位；它是 **OpenClaw** 概念（`registerContextEngine`）。DSH 半边实际用 `systemPrompt` 服务 + `agent/pre-step` 事件（`agent.ctx.on("agent/pre-step", compactBeforeStep, { prepend: true })`） | 不改变我们的设计（本来就是 Assemble 阶段插入），但**别再对外引用"DSH 的 contextEngine"** |
| `dsh plugin add <spec>` 只是转发给 pnpm | ⚠️ 不止：`runProfilePnpm()` 跑完后 `reconcile()` 会 diff 依赖、读新增包的 `dsh.bundle`，**自动把包名写入 profile `package.json` 的 `dsh.profile.bundles`**；无 `dsh.bundle` 的包仅警告 *"installed as a plain dependency, not a profile layer"* | 这是"清单即能力声明"的实物证据：**宿主只读清单即可枚举插件，无需载入代码**——正是 `voices.rs` 目录探测思路的泛化 |

### 11.2 值得照搬的 8 条（语言无关）

1. **清单即能力声明**：清单声明 patch 路径 + `platform/inject` + 能力；宿主读清单即可枚举插件。
2. **分层 patch 组合**：插件树 = 有序层叠加（bundle → profile 用户层 → 命令行层），每层是 `id` 索引的条目表；`insert` 追加、`id` 命中则**整体替换 config（不做深合并，必须重述全部字段）**、`disabled` 控制激活。Rust 用 TOML/YAML + `serde` 即可复刻。
3. **注册表 + 命名空间**：工具/命令/provider/槽位都"按名注册进某 scope，**重复注册直接报错**"，杜绝隐式覆盖（`dsh-persona` README 亦强调全局挂载会 *fails loud*）。
4. **声明式依赖注入**：`inject = [service...]`，全就绪才激活；任一消失即卸载、回归即重载——**热重载不需要专门的 reload 机制**。
5. **资源统一登记为 effect → disposer**：逆序、可 await、幂等；卸载/重配/进程退出共用一条清理路径（对应 Rust 的 `Drop` / `CancellationToken`）。
6. **激活状态机显式化**：`PENDING → LOADING → ACTIVE / FAILED → UNLOADING → DISPOSED`，且 `enabled`（保存的选择）与 `fiberPhase`（实际激活态）**分开上报**，前端才能显示「已启用但加载失败」（已并入 §6 映射表）。
7. **密钥与配置分离**：config 只存 `apiKeyEnv` 这类**引用**，值走独立 credential store（`@deepseek-ai/dsh-credentials`）。
8. **schema 驱动校验**：`Config` = schema，激活前校验（失败抛 `ValidationError` 拒绝加载），可导出 JSON Schema 供 GUI；`.volatile()` 标记可热改字段。

### 11.3 不必照搬（Node / Cordis 特有）

Proxy 原型链式 `ctx` 服务解析、`isolate` 用 `Symbol` 做 realm 标签、YAML 里执行 `!!js` 任意 JS 表达式、ESM 动态 `import()`、浏览器 `window.__ModuleLoader__` + `/plugins` 组合脚本 + 客户端 HMR 版本戳、以 pnpm 作安装器及 `reconcile` 对账、`engines.dsh` SemVer 兼容豁免、typert 装饰器式远程方法自动生成（Rust 用 serde + 显式 RPC trait 更直白）。

### 11.4 本次未找到证据的两项（**不要当既有事实引用**）

- DSH 侧**不存在** `contextEngine` 槽位；
- **没有**由 schema 自动生成设置表单的随发布客户端（`dsh-settings` README：`autoGenerate` 尚无客户端使用）。

---

## 12. 一句话回答「怎么给 LLM 加灵魂与记忆」

> **灵魂** = 一份结构化、可编辑、可热切换的人格档案（`[soul]`），在 LLM 调用前组装进 `instructions`；
> **记忆** = 一个 `MemoryProvider` 插件（默认 `NoopMemory`，图实现按 graph-memory 的 schema 与算法在 Rust 里落地），在关键路径前置召回、在后台异步抽取写入；
> **管理界面** = 新增「灵魂」「记忆」两个 tab，由服务端 JSON Schema 提供字段元数据与校验（表单仍手写——DSH 的 `autoGenerate` 至今无客户端使用，别信"自动生成表单"）；
> 至于 graph-memory 上游仓库：它在架构上值得**完整借鉴**（导航层 + 原文证据、抽取隔离、召回预算、保留策略），但在集成上**没法直接调用**——它只存在于宿主进程内。
