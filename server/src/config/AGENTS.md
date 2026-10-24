# 配置来源、规范化与存储

## 当前实现（以当前工作树为准）

[入口](<../main.rs>)装配配置存储：**SQLite 配置库是唯一后端**（`SqliteConfigStore`，见 [store](<store.rs>)），DB 路径解析优先级：`XIAOZHI_CONFIG_DB` → `--config-db` → `/data/data.db`。空库（首次安装）自举写入内置默认配置后启动；外置 config.toml 文件配置逻辑（含首启 bootstrap 导入）已于 2026-10 移除，SQLite 打不开则进程启动即失败（配置是必需能力，无降级）。XIAOZHI_ADMIN_DIR 只覆盖本次运行，不写回，GET 配置仍显示原值。

[ConfigStore](<store.rs>) 唯一实现 `SqliteConfigStore`：读、写、overlay、预设、location、只读查询与备份经同一抽象。**远端访问**：`POST /api/config/db/query` 提供只读 SQL 查询（用户可用数据库客户端形态远程查库），`POST /api/config/db/backup` 提供 `VACUUM INTO` 一致性备份。迁移设计见[迁移设计](<store/AGENTS.md>)（其中文件/默认后端、bootstrap 导入等描述为历史设计，现状以本节与源码为准）。

- `cfg_documents`（整份 Config blob）+ `cfg_device_overlays` + `cfg_presets` + `cfg_revisions`（历史/回滚）。`location()` = DB 文件路径，用于挂载卷持久化判定与 hot-audio 落点派生。
- DB 路径必须在挂载卷内；不要把落点随意固定到容器层，容器重建会丢数据。memory 的实际路径以其配置为准，不假定所有路径必然同目录。

## 兼容与字段修改

[配置聚合](<../config.rs>)只聚合 server/audio 与插件配置；能力私有配置靠近各插件。

- `engine` 选实现，TTS 私有参数进 `[tts.<id>]`，speed/speaker 等 trait 入参留本体。LLM/ASR/VAD 当前单实现参数仍在本体，不机械搬进不存在的子段。
- 旧 backend、TTS 扁平参数继续双读，normalize 幂等且新位置优先；保存只写新结构。旧客户端补丁也必须重写，而非只迁移文件。
- 未知 engine 显式报错并列出可选值；memory none 是合法停用，不是未知值。声明 id 与描述符一致性由测试守护。
- 新字段同步[配置样例](<../../config.example.toml>)、[注册表元数据](<../plugins/registry/AGENTS.md>)、客户端映射与往返测试；不要在文档复制默认值/热生效表。
- [overlay](<overlay.rs>)只合并白名单会话参数，生效配置=全局规范化配置⊕设备增量；全局引擎构建不受影响。

## 默认值与存量配置对齐（[defaults.rs](<defaults.rs>)）

配置库里存的是**显式值**，serde 的 `#[serde(default)]` 只对"键缺失"生效——所以把某个默认值从
`false` 改成 `true`，老库（含"重装系统但保留 `./server-data` 卷"的场景）仍会是 `false`。这一层就是
为它准备的：

- `CURRENT_DEFAULTS_VERSION` + `BATCHES`：默认值变更**版本化登记**。每次改默认值就往
  `BATCHES` 追加一批（版本 +1），已发布批次永不修改；`STANDARD_BEHAVIOR_ON` 是 v1 的
  "标准行为默认开启"清单。
- 启动时 `SqliteConfigStore::align_defaults()` 按库里 `cfg_documents.defaults_version` 对齐，
  三条规则：键缺失/`null` → 补新默认；**值等于旧默认** → 视为没动过、对齐到新默认；值不等于旧默认
  → 用户显式自定义、原样保留（`kept` 只记日志）。写库留 `cfg_revisions(source="defaults-migration")`
  可回滚，版本推进后不再重复执行。
- 只处理 `cfg_documents`：设备 overlay 只在用户显式保存时才有内容（没有"旧默认整份文档"），
  预设是用户主动快照，动它等于篡改。
- 改默认值的完整动作：① 插件 `Default` + serde `default_*()`（单一权威）；②
  [注册表字段](<../plugins/registry/AGENTS.md>) `default_hint`；③ 客户端表单状态初值
  （`client/shared/.../Config*State.kt`）；④ `BATCHES` 追加登记 + `CURRENT_DEFAULTS_VERSION` +1；
  ⑤ [配置样例](<../../config.example.toml>)。漂移由 `defaults/tests.rs` 的
  `targets_match_code_defaults` 与 `registry/tests.rs` 兜底。
- 部署核对：升级后启动日志会打印一行 `配置库默认值对齐 v0 → v1：…`（写明哪些键被打开、
  哪些按用户自定义保留）；`docker compose logs` 可见。库内进度可查
  `SELECT defaults_version FROM cfg_documents`，逐项现值可查
  `POST /api/config/db/query` → `SELECT json_extract(doc,'$.wakeup.enabled') ...`。

本级说明 · [HTTP 契约](<../gateway/ws/config/AGENTS.md>) · [配置测试](<tests.rs>) · [存储测试](<store/sqlite/tests.rs>)


---

## 配置使用、预设与设备 overlay

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 3. 配置

配置库路径优先级：**`XIAOZHI_CONFIG_DB` 环境变量** → **`--config-db <path>` 参数** → **`/data/data.db` 兜底**；空库自举写入内置默认（`/data/models` 生产路径）并启动。

```bash
## 方式一：环境变量
export XIAOZHI_CONFIG_DB=$PWD/server-data/data.db
cd server && ~/.cargo/bin/cargo run --features sherpa

## 方式二：命令行参数
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config-db /tmp/data.db
```

关键项（完整字段见 `config.example.toml`）：

- `server.port`（默认 8000，恒监听 `0.0.0.0`）；`server.expected_token`（Bearer，空 = 不校验）。
- `audio.downlink_sample_rate` / `downlink_frame_duration_ms`：下行协商参数；`binary_protocol_version` 建议 1 真机验证后再切 2/3。
- `audio.downlink_buffer_ms` / `downlink_buffer_start_ms` / `downlink_lead_ms`：下行缓冲三段式（详见 example 注释，重启生效）。
- `llm.api_base` / `api_key` / `model` / `system_prompt`：OpenAI 兼容 Responses API。
- `tts.engine`：`kokoro`（本地 INT8，默认，参数进 `[tts.kokoro]` 子段）或 `xfyun`（在线，凭据进 `[tts.xfyun]`）。旧写法（`backend="sherpa"` + `[tts]` 直挂模型路径）加载时自动迁移，保存一次写成新结构。改后**新会话/测试台立即热切换**。
- `command.*` / `wakeup.*` / `memory.*` / `soul.*` / `aiui.*`：见[可选上下文功能](<../plugins/AGENTS.md#可选上下文功能>)。

---

### 4. 配置预设与每设备 overlay（存于配置库）

预设与 overlay 均为配置库中的表（`cfg_presets` / `cfg_device_overlays`）：

| 表 | 内容 | 说明 |
|---|---|---|
| `cfg_presets` | 全局配置快照（按名） | 管理台「配置 → 预设」保存/应用/删除；应用 = 复用 `PUT /api/config` 部分更新管线（含密钥护栏与生效粒度回报） |
| `cfg_device_overlays` | 单设备增量 | 仅白名单会话内字段（音色/语速/音色语言、提示词与历史长度、人格、唤醒话术、指令词表、记忆召回参数）；`server`/`audio`/引擎选择等结构型字段不可覆盖（越界写入被拒并给出「怎么修」提示）；无 overlay 的设备行为与全局完全一致 |

生效公式：`生效配置(设备) = 默认值 ⊕ 配置库全局配置 ⊕ 设备 overlay`。overlay 引擎不受影响：引擎仍只按全局配置构建（overlay 永不触发引擎重建）。

```bash
curl http://<host>:8000/api/devices/<device_id>/overlay        # 查看设备 overlay（无则 {}）
curl -X PUT -H "Content-Type: application/json" \
     -d '{"tts":{"speaker":3,"speed":1.2}}' \
     http://<host>:8000/api/devices/<device_id>/overlay        # 该设备换音色/语速（新会话生效）
curl -X POST http://<host>:8000/api/config/presets -d '{"name":"night"}'   # 当前配置快照为预设
curl -X POST http://<host>:8000/api/config/presets/night/apply             # 应用预设
```

> ⚠️ **容器部署**：`data.db`（配置+记忆共用的 SQLite 库，含预设与 overlay）、`hot-audio/`、`rag-docs/` 都由 DB 路径所在目录派生——**DB 路径必须放在挂载卷内**，否则重建容器即丢预设/overlay/记忆/问候音频。

---

## 上级入口

[上级 AGENTS](<../../AGENTS.md>)

## 直属下级

- [store](<store/AGENTS.md>)
