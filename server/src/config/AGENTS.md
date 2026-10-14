# 配置来源、规范化与存储

## 当前实现（以当前工作树为准）

[入口](<../main.rs>)装配配置存储后端：`XIAOZHI_CONFIG`/`--config` 仅用于**兼容导入旧 TOML** 与读取 `[memory].db_path`；**真源是 SQLite 配置库**（`SqliteConfigStore`，见 [store](<store.rs>)），DB 路径解析优先级：`XIAOZHI_CONFIG_DB` → `--config-db` → `[memory].db_path` → `/data/xiaozhi.db`。空库且有旧 TOML 时由 `load_config_inner` **兼容导入**（`source=bootstrap`），导入后 DB 优先、旧 TOML 不再被读取。SQLite 打开失败则降级到文件/默认后端（保证进程总能起来）。XIAOZHI_ADMIN_DIR 只覆盖本次运行，不写回，GET 配置仍显示原值。

[ConfigStore](<store.rs>)现有 `SqliteConfigStore`（默认真源）、`FileConfigStore`（降级）与 `DefaultConfigStore`：读、写、overlay、预设、location 经同一抽象。**远端访问**：`POST /api/config/db/query` 提供只读 SQL 查询（用户可用数据库客户端形态远程查库），`POST /api/config/db/backup` 提供 `VACUUM INTO` 一致性备份。SQLite 迁移设计见[迁移设计](<store/AGENTS.md>)，**已于 2026-10-14 落地 P1 + 主配置/overlay/预设全进库**。

- SQLite 后端：`cfg_documents`（整份 Config blob）+ `cfg_device_overlays` + `cfg_presets` + `cfg_revisions`（历史/回滚）。`location()` = DB 文件路径，用于挂载卷持久化判定与 hot-audio 落点派生。
- 文件后端实时读盘（仅降级路径）；解析失败是否回退由调用方决定。location=None 表示没有外部来源，配置保存返回 400，热句音频禁用磁盘持久化。
- TOML 先 normalize 再写同目录临时文件并 rename；单文件 bind mount 不能原子替换时回退原地写，风险与告警保留。生产推荐可写目录挂载。
- 预设、overlay、hot-audio 跟配置来源目录派生；不要随意固定到容器层，容器重建会丢数据。memory 的实际路径以其配置为准，不假定所有路径必然同目录。

## 兼容与字段修改

[配置聚合](<../config.rs>)只聚合 server/audio 与插件配置；能力私有配置靠近各插件。

- `engine` 选实现，TTS 私有参数进 `[tts.<id>]`，speed/speaker 等 trait 入参留本体。LLM/ASR/VAD 当前单实现参数仍在本体，不机械搬进不存在的子段。
- 旧 backend、TTS 扁平参数继续双读，normalize 幂等且新位置优先；保存只写新结构。旧客户端补丁也必须重写，而非只迁移文件。
- 未知 engine 显式报错并列出可选值；memory none 是合法停用，不是未知值。声明 id 与描述符一致性由测试守护。
- 新字段同步[配置样例](<../../config.example.toml>)、[注册表元数据](<../plugins/registry/AGENTS.md>)、客户端映射与往返测试；不要在文档复制默认值/热生效表。
- [overlay](<overlay.rs>)只合并白名单会话参数，生效配置=全局规范化配置⊕设备增量；全局引擎构建不受影响。

本级说明 · [HTTP 契约](<../app/ws/config/AGENTS.md>) · [配置测试](<tests.rs>) · [存储测试](<store/tests.rs>)


---

## 配置使用、预设与设备 overlay

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 3. 配置

配置加载优先级：**`XIAOZHI_CONFIG` 环境变量** → **`--config <path>` 参数** → **内置默认（/data/models 生产路径）**。

```bash
## 方式一：环境变量
export XIAOZHI_CONFIG=$PWD/server/config.toml
cd server && ~/.cargo/bin/cargo run --features sherpa

## 方式二：命令行参数
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
```

关键项（完整字段见 `config.example.toml`）：

- `server.port`（默认 8000，恒监听 `0.0.0.0`）；`server.expected_token`（Bearer，空 = 不校验）。
- `audio.downlink_sample_rate` / `downlink_frame_duration_ms`：下行协商参数；`binary_protocol_version` 建议 1 真机验证后再切 2/3。
- `audio.downlink_buffer_ms` / `downlink_buffer_start_ms` / `downlink_lead_ms`：下行缓冲三段式（详见 example 注释，重启生效）。
- `llm.api_base` / `api_key` / `model` / `system_prompt`：OpenAI 兼容 Responses API。
- `tts.engine`：`kokoro`（本地 INT8，默认，参数进 `[tts.kokoro]` 子段）或 `xfyun`（在线，凭据进 `[tts.xfyun]`）。旧写法（`backend="sherpa"` + `[tts]` 直挂模型路径）加载时自动迁移，保存一次写成新结构。改后**新会话/测试台立即热切换**。
- `command.*` / `wakeup.*` / `memory.*` / `soul.*` / `aiui.*`：见[可选上下文功能](<../plugins/AGENTS.md#可选上下文功能>)。

---

### 4. 配置预设与每设备 overlay（落盘布局）

与主 `config.toml` 同目录：

| 目录 | 内容 | 说明 |
|---|---|---|
| `presets/<name>.toml` | 全局配置快照 | 管理台「配置 → 预设」保存/应用/删除；应用 = 复用 `PUT /api/config` 部分更新管线（含密钥护栏与生效粒度回报） |
| `overlays/devices/<device_id>.toml` | 单设备增量 | 仅白名单会话内字段（音色/语速/音色语言、提示词与历史长度、人格、唤醒话术、指令词表、记忆召回参数）；`server`/`audio`/引擎选择等结构型字段不可覆盖（越界写入被拒并给出「怎么修」提示）；无 overlay 的设备行为与现状完全一致 |

生效公式：`生效配置(设备) = 默认值 ⊕ config.toml（全局） ⊕ 设备 overlay`。overlay 引擎不受影响：引擎仍只按全局配置构建（overlay 永不触发引擎重建）。

```bash
curl http://<host>:8000/api/devices/<device_id>/overlay        # 查看设备 overlay（无则 {}）
curl -X PUT -H "Content-Type: application/json" \
     -d '{"tts":{"speaker":3,"speed":1.2}}' \
     http://<host>:8000/api/devices/<device_id>/overlay        # 该设备换音色/语速（新会话生效）
curl -X POST http://<host>:8000/api/config/presets -d '{"name":"night"}'   # 当前配置快照为预设
curl -X POST http://<host>:8000/api/config/presets/night/apply             # 应用预设
```

> ⚠️ **容器部署**：`presets/`、`overlays/`、`hot-audio/`、`memory.db`、`rag-docs/` 都在配置文件所在目录下派生——**配置文件必须放在挂载卷内**，否则重建容器即丢预设/overlay/记忆/问候音频。

---

## 上级入口

[上级 AGENTS](<../../AGENTS.md>)

## 直属下级

- [store](<store/AGENTS.md>)
