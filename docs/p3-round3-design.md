# P1-5 配置预设 + 每设备 overlay（第 3 轮设计）

> 版本：v1.0　日期：2026-10-12
> 上游：`docs/p1-features-prd.md` §3.1（P1-5）
> 决策基线：用户拍板「**保守白名单——不包含 engine 切换**」，且本项动 `Config::load`/保存核心链路、回归风险最高，单独成轮。

---

## 0. 核心冲突与解法（先讲结论）

**冲突**：`Engines::live_config()` 返回进程级**单一全局**快照 `host.config()`（会话开始由 `refresh_from_disk` 从磁盘刷新），而每设备 overlay 要求"每台设备有自己的生效配置 = 全局 ⊕ 该设备 overlay"。直接在 `PluginHost` 里塞 per-device 快照会污染全局状态、且与"共享引擎只按全局构建"的契约打架。

**解法（不引入 per-device 全局状态）**：
- `live_config()` 保持全局，引擎（ASR/TTS/LLM/Memory）**只按全局配置构建**——overlay 永不触碰引擎签名。
- 新增 `Engines::effective_config(device_id) -> Arc<Config>` = `merge_overlay(全局 live_config, overlays/devices/<device_id>.toml)`，**逐设备按需计算**。
- 会话在 `start.rs` 算一次 `eff_config` 存入 `Session`，两类消费点切换数据源：
  - **请求时现读类**（reply.rs / session.rs / wakeup.rs 各处）→ 直接改用 `self.eff_config`；
  - **会话开始快照类**（`start.rs:77` command 闸门、`start.rs:79` wakeup 问候）→ 这两处数据源从 `engines.live_config()` 换成 `self.eff_config`。
- **结构型参数**（server/audio/aiui/real_audio，restart-only）一律保留 `engines.live_config()` 读取，绝不进 overlay。

理由：引擎签名函数（`signature_of` / `sig_llm` 等）是**纯函数**、只吃传入的 `&Config`；而 `effective_config`（每设备）**永不喂给引擎构建**——引擎只按 `live_config()`（全局）构建，overlay 因此绝不触碰共享引擎实例。白名单绝大多数字段本就不在任何签名里；个别字段（如 `llm.system_prompt` 出现在 `sig_llm`）即便恰好在签名内，也只影响"从**全局**配置算出的全局签名"，绝不会让某台设备的 overlay 触发共享引擎重建（已确认：签名仅用于全局引擎热切换判定，从不按每设备 effective_config 计算）。可见测试 `merge_overlay_is_pure_and_session_internal_only`。

---

## 1. 生效公式与白名单（单一事实来源）

```
生效配置(device) = 默认值 ⊕ config.toml（全局） ⊕ device_overlay(device_id)   // 仅白名单字段
```

`device_overlay` 白名单（**显式枚举**，保守、不含 engine 切换；任何不在表内的路径写入即被拒并给「怎么修」中文错误）：

| 段 | 允许的白名单路径 | 排除（原因） |
|---|---|---|
| `tts` | `speaker`、`speed`、`xfyun.voice` | `engine`、`kokoro.*`（含 `kokoro.lang`——Kokoro 的 `lang` 烤死在共享引擎池 `kokoro.rs:31`，逐设备覆盖须重建共享引擎，违反契约）、`cache_entries`（跨实现通用、影响缓存语义） |
| `llm` | `system_prompt`、`max_history` | `model`/`api_base`/`api_key`/`temperature`/`stream`（切换 LLM 实现 / 凭证，必改引擎实例）。注：`system_prompt` 虽也出现在 `sig_llm`（引擎签名）里，但引擎只按**全局**配置构建、`effective_config` 绝不喂引擎，故放白名单安全 |
| `soul` | 整个 `[soul]` 段 | 无引擎影响 |
| `wakeup` | 整个 `[wakeup]` 段 | 仅 greeting 话术 |
| `command` | 整个 `[command]` 段 | 仅 keywords/reply |
| `memory` | `recall_enabled`、`fresh_turn_count`、`recall_max_nodes`、`recall_max_tokens`、`recall_budget_ms`、`inject_position` | `engine`/`enabled`/`db_path`/`db_busy_timeout_ms`/`extraction_enabled`/`maintenance_interval`/`scope_prefix`/`quarantine_max_attempts`/`retention`/`extractor`/`rag`（引擎构建或共享存储行为） |

> **密钥处理**：白名单天然不含任何 `SECRET_PATHS` 字段（`llm.api_key` 等已被排除），因此 overlay 不可能写入密钥；PRD 要求的"密钥只回 presence、空串保留"在 overlay 层被白名单天然满足，无需额外 redact 分支（GET overlay 不返密钥）。
> **显式 `null`**：overlay 中某字段显式 `null` = 清除该字段回落默认（与 `merge_client_patch` 的 `merge_patch` 语义一致）。
> **未知键**：TOML 无 `deny_unknown_fields`，故白名单校验是**唯一**拦截拼错字段的机制——必须在 merge 前按路径校验。

---

## 2. 合并实现（复用既有范式）

新增模块 `server/src/app/ws/overlays.rs`：

- `fn overlay_dir(config_path: &str) -> PathBuf`：`Path::new(config_path).parent().join("overlays")`（复用 `engine.rs:118` hot-audio 同款基目录推导）。
- `fn device_overlay_path(config_path: &str, device_id: &str) -> PathBuf`：`overlay_dir/<sanitized device_id>.toml`。`device_id` 做文件名 sanitize（拒绝 `/`、`.`、空），防路径穿越。
- `fn device_overlay_path(config_path: &str, device_id: &str) -> Option<PathBuf>`（overlay.rs）：`config_path` 同目录 `overlays/devices/<sanitized>.toml`；`device_id` 经 `sanitize_device_id` 校验，非法 → `None`。
- `fn read_device_overlay_value(path: &Path) -> Option<Value>`（overlay.rs）：读 TOML → JSON；文件不存在 / 解析失败 → `None`（= 用全局，防御：坏文件不阻断会话）。
- `fn validate_overlay(overlay_json: &Value) -> Result<(), String>`（overlay.rs）：递归遍历 JSON 树，每个叶子/对象路径查白名单表（`overlay_allowed(path) -> bool`）；首个越界路径返回中文错误：`字段 <path> 不允许在设备 overlay 中覆盖（只可覆盖会话内参数：tts 音色/语速、llm 提示词与历史长度、soul 人设、wakeup/command 话术、memory 召回参数；结构型参数 server/audio 等不可覆盖）。请用 GET /api/config 修改全局配置。`。
- `fn merge_overlay(base: &Config, overlay: &Value) -> anyhow::Result<Config>`（overlay.rs）：复用既有 `merge_patch`（`app/ws/config.rs` 原有 + 因 `config` 是比 `app::ws` 更底层模块、overlay 不能反向依赖，故在 `config.rs` 新增 `pub(crate)` 副本；语义完全一致：JSON 递归合并、显式 `null`=清除）+ `Config::normalize()`。先把 base 转 JSON → `merge_patch` → 解析回 `Config` → `normalize()`。与 `merge_client_patch` 同范式（白名单已排除密钥，故不 sanitize）。
- `Engines::effective_config(&self, device_id) -> Arc<Config>`（engine.rs 新增）：
  - `global = self.host.config()`；
  - `device_id == "unknown"` 或 `config_path` 为 `None` 或 overlay 文件不存在 → 返回 `global.clone()`；
  - 否则 `merge_overlay(&global, &overlay)` → `Arc::new(merged)`。

> **不缓存 overlay 文件**：overlay 小且在会话开始时算一次（存入 `Session.eff_config`），请求时不再重读盘。后续若需热刷新可加 mtime 失效缓存，本期不做。

---

## 3. 端点设计（不改动 `PUT /api/config` 语义）

### 3.1 设备 overlay（新增，与 P1-2 `device_inject` 并列）
路由（`app/ws/mod.rs`）：
```
.route("/api/devices/{device_id}/overlay", get(device_overlay).put(set_device_overlay).delete(delete_device_overlay))
```
- `GET`：返回该设备当前 overlay（白名单字段子集；无 overlay 文件返回 `200 {overlay:{}}`）。`config_path == None`（内置默认）→ `400`（与 PUT /api/config 一致）。
- `PUT`：请求体 = 白名单字段的部分 JSON；`validate_overlay_paths` 校验 → 不合规则 `400` 含「怎么修」；通过则 `merge_overlay(全局, 提交)` 后写 `overlays/devices/<id>.toml`（复用 `persist_config` 的原子写 `config.rs:269`）。返回成功 + `impact`（取白名单字段最严热生效档 = NextSession）。
- `DELETE`：删文件（不存在也算成功 `200`）。
- 鉴权复用全局 `auth_ok`（与 `device_inject` 一致）。

### 3.2 配置预设
路由：
```
.route("/api/config/presets", get(list_presets).post(save_preset))
.route("/api/config/presets/{name}", delete(delete_preset))
.route("/api/config/presets/{name}/apply", post(apply_preset))
```
- `presets/<name>.toml` 落盘于 `config_path.parent()/presets/`（同名 sanitize）。
- `GET /api/config/presets`：列出预设名（目录扫描）。
- `POST /api/config/presets`（body `{name, ...}` 或仅 `{name}`）：「保存当前为预设」——以**当前盘上配置**（`current_config`，config.rs:37）全量序列化写入 `presets/<name>.toml`（本地文件可含密钥，与 config.toml 同口径）。`name` 缺失/非法 → `400`。
- `POST /api/config/presets/{name}/apply`：「应用预设」= 读 `presets/<name>.toml` → `merge_client_patch(当前盘配置, 预设JSON)` → `persist_config` 写回主配置。**复用 `PUT /api/config` 的同一管线语义**（部分更新 + 原子写 + `sanitize_patch` 密钥护栏 + `save_impact` 回报）。热生效惰性发生于下一次会话开始（`refresh_from_disk`），与手动保存一致。
- `DELETE /api/config/presets/{name}`：删预设文件。
- `config_path == None` → 预设/应用均 `400`。

> 验收 §3.1-4「应用预设后 GET /api/config 等于预设内容（密钥 redact）」：应用走 `merge_client_patch`，密钥空串保留原值；GET 侧 `redact_secrets` 摘掉密钥返 `has_` 标记——与手动保存同口径，天然满足。

---

## 4. 会话层接入（编辑点清单）

- `Session` 结构体（session.rs）新增字段 `eff_config: Arc<Config>`。
- 会话构造处（start.rs / run_session）在 `refresh_from_disk` 之后调用 `engines.effective_config(&device_id)` 存入 `eff_config`。
- 数据源切换（白名单消费点 → `self.eff_config`；结构型 → 保留 `engines.live_config()`）：
  - ✅ `start.rs:77` `CommandGate::new(&eff.command)`（原 `&engines.live_config().command`）
  - ✅ `start.rs:79` `WakeGreeting::new(&eff.wakeup)`
  - ✅ `reply.rs:41` `let cfg = self.eff_config.clone();`（soul/memory/llm.system_prompt 全白名单）
  - ✅ `reply.rs:309` / `session.rs:502` `self.eff_config.llm.max_history`
  - ✅ `reply.rs:382-389` `let live = self.eff_config.clone();`（hot_audio 取 speaker/speed）
  - ✅ `wakeup.rs:133` 改用 `self.eff_config`
  - ⛔ `start.rs:57` `real_audio()`、`start.rs:61-68` aiui、`reply.rs:481` `audio.max_lead_ms()` 保持 `engines.live_config()`（结构型）
- 缓存键联动：`HotAudioKey::from_session`（hot_audio.rs:112）已把 speaker/speed 编入键，overlay 改音色自动重合成，无需额外失效逻辑。

---

## 5. 前端触点（Kuikly commonMain，已实现）

### 5.1 关键约束：Kuikly NetworkModule 无 PUT/DELETE

实测（core-macosArm64 2.28.0 klib 反查）`NetworkModule` 只有 `requestGet` / `requestPost` / `requestPostBinary` / `httpRequest`（POST + body）/ `httpRequestBinary`。**没有 PUT/DELETE**。因此按 OTA 固件删除的既有惯例（`POST /api/ota/firmware/delete`），服务端为客户端补了 POST 别名路由（`app/ws/mod.rs`，REST 语义端点原样保留）：

| REST 端点 | 客户端实际调用 |
|---|---|
| `PUT /api/devices/{id}/overlay` | `POST /api/devices/{id}/overlay`（body 同 PUT） |
| `DELETE /api/devices/{id}/overlay` | `POST /api/devices/{id}/overlay/delete` |
| `DELETE /api/config/presets/{name}` | `POST /api/config/presets/{name}/delete` |

### 5.2 编辑对象与差量 overlay（`ConfigFormState.kt`）

- `editDeviceId`（observable）："" = 全局；非空 = 设备 ID。`load`/`save` 据此分道。
- 设备加载（`loadDeviceTarget`）：先 `GET /api/config` 填表（全局基线）→ 抓 `buildOverlayJson()` 快照（`deviceBaseline`）→ `GET /api/devices/{id}/overlay` 后复用 `fill()` 套用（overlay 只含白名单字段，`fill` 缺键保留当前值 → 表单即设备生效值）。
- 设备保存（`saveDeviceOverlay`）：`computeOverlayDiff()` 把当前表单与 `deviceBaseline` 按 section（`tts/llm/soul/wakeup/command/memory`）求差（同构建器同序 put → `toString` 相等即相等），只把有改动的 section 写入 overlay；**差量为空 → 清除 overlay**（恢复全局）。基线只在加载时抓取、保存成功后不更新——保证"保存后再保存"不会把上一次差量误判为空而清掉 overlay。
- `backToGlobal` / `clearDeviceOverlay`：切回全局编辑 / 删 overlay 后切回。

### 5.3 预设（`ConfigFormState.kt` + `ConfigCards.kt`）

- 「保存为预设」只上报 `{name}`：服务端读盘快照当前生效配置（表单草稿可能落后于盘上值）。
- 应用成功后自动 `load()` 重载表单；删除走 POST 别名。
- 预设仅作用于**全局**配置：设备个性化模式下预设卡显示不可用提示。

### 5.4 UI 落点

- `ConfigSections.kt`：新增置顶 section `presets`（order 5）。
- `ConfigCards.kt`：`renderConfigSection` 新增 `presets` 分支；`editTargetBanner`（设备目标时显示覆盖范围说明 + 「返回全局」/「恢复全局配置」）；`presetCard`（名称输入 + 保存 + 列表逐项应用/删除）。
- `ConfigPage.kt`：支持 `?edit_device=<id>` URL 参数；标题 `配置 · 设备 <id>`。
- `AdminPage.kt` / `AdminShell.kt`：标题与保存按钮文案随 `editDeviceId` 切换（「保存配置」⇄「保存设备配置」）。⚠️ `primaryButton` 文案是构建期参数，必须用**两个 vif 块**切换两份按钮（observable 只在 vif 闭包内读取才触发重建），不能内联三元。
- `OpsPages.kt`：`renderOpsDevices` 新增 `form` + `onEditDevice` 参数，设备卡加「个性化配置」按钮（测试台连接不显示）→ 置 `editDeviceId` + `load()` + 跳配置 section。
- `ConnectState.kt`（macOS）：连接成功后 `form.loadPresets(shell, base)`。
- `ConfigContextState.kt`：`soulJson()` private → internal（overlay 构建 `[soul]` 全段复用，与保存/预览同一构建避免字段漂移）。

---

## 6. 任务拆分（R3-1 … R3-10）

| ID | 任务 | 落点文件 |
|---|---|---|
| R3-1 | overlay 白名单表 + `overlay_allowed` + `validate_overlay` | `config/overlay.rs`（新） |
| R3-2 | `device_overlay_path` / `read_device_overlay_value` / `merge_overlay` / 路径 sanitize / 落盘目录推导 | `config/overlay.rs` |
| R3-3 | `Engines::effective_config(device_id)` | `engine.rs` |
| R3-4 | `Session.eff_config` + 编辑点切换（§4） | `app/session.rs` / `start.rs` / `reply.rs` / `wakeup.rs` |
| R3-5 | 设备 overlay 端点 GET/PUT/DELETE + 路由 | `app/ws/config.rs` + `app/ws/mod.rs` |
| R3-6 | 预设端点 list/save/apply/delete + 路由 | `app/ws/config.rs` + `app/ws/mod.rs` |
| R3-7 | overlay 单测（白名单拒止 / merge 幂等 / null 清除 / 纯函数不改全局 / 目录 sanitize） | `config/overlay/tests.rs` |
| R3-8 | preset 端点单测（save/apply/delete 落盘 + apply 复用 merge_client_patch 语义） | `app/ws/config/tests.rs`（新增用例） |
| R3-9 | 前端：预设卡 + 当前编辑对象指示器 + 设备「个性化配置」入口 | `client/.../Config*.kt` / 设备页 |
| R3-10 | 全量回归：cargo 双 feature check + cargo test filter + gradle 双端编译 | — |

---

## 7. 验收映射（PRD §3.1）

1. 设备 A 设 `tts.speaker=X` → A 新会话用 X；B 未设 → 全局值；删 A overlay → 回全局。✅ R3-2/3/4
2. overlay 字段热生效粒度与全局一致（`tts.speed` 新会话生效；`server.port` 不可覆盖，写越界返回错误）。✅ R3-1/5
3. 重启后预设列表与 overlay 文件仍在（落盘）。✅ R3-2/6
4. 应用预设「夜间模式」后 GET /api/config == 预设内容（密钥 redact）。✅ R3-6
5. overlay 写入不存在字段路径 → 加载/保存失败含「怎么修」。✅ R3-1
6. 密钥护栏回归：overlay 提交空串密钥被白名单拒（密钥本不在白名单）。✅ R3-1（含集成测试）
