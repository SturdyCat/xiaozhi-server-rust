# SQLite 配置迁移设计（提案，未落地）

[ConfigStore](<../store.rs>) 当前只有 FileConfigStore 与 DefaultConfigStore：读、写和 location 经同一抽象；**尚无 SQLite 配置后端、config-db 启动参数或历史/备份端点**。本文是迁移方案与论证，不是已支持的运行契约。

> **历史设计 / 提案，未落地**：以下正文保留原章节编号及当时的结论；文中 `§N` 即本文同号标题。评估实施时应重新核对现有 ConfigStore、部署路径与数据迁移边界。

原题：配置迁移到 SQLite 设计（config → SQLite，与 memory 共库）。

> 回答的问题：**既然已经引入 rusqlite，能否把配置迁移到 SQLite，并提供接口让 web 与 mac app 管理配置，且与 memory 共用同一个 db？**
>
> 结论：**可以，且推荐做**。但真正的价值与风险都不在"把 TOML 换成一张表"，而在三件事上：
> ① 存储后端**抽象**（`ConfigStore` trait）而非硬替换；② 与 memory 共用库文件时的**迁移命名空间隔离**（不隔离会静默丢表，见 §3.1）；
> ③ **bootstrap 引导源 + TOML 导入/导出**，否则会出现新一轮"配置到底在哪"的困惑。
>
> 另外有一个容易被忽略的事实：**"提供接口，用 web 与 mac app 管理配置"这件事现状已经完成**——
> `client/shared/commonMain` 的 `ConfigFormState.kt` 已经经 `NetworkModule` 走 `GET/POST /api/config`，
> web（h5App）与 mac app 共用同一份 `commonMain` 代码。本次要迁移的是**接口背后的存储**，HTTP 契约保持不变，
> 因此客户端在 P0/P1 阶段**零改动**。

---

## 1. 现状事实清单（迁移面）

| 关注点 | 现状 | 权威位置 |
|---|---|---|
| 配置从哪来 | `XIAOZHI_CONFIG`（env）→ `--config <path>` → 内置 `Config::default()`（`/data/models` 生产默认值） | `server/src/main.rs:121`、`:134`；[配置说明](<../AGENTS.md>)（原 `agents.md` §8） |
| 读（HTTP） | 每次请求**实时读盘**（坏文件则回退内存快照，保证管理页打得开） | `server/src/app/ws/config.rs:39` |
| 读（会话/测试台） | 每次会话开始 `refresh_from_disk()` 读盘 → 签名比对 → 热重建 TTS/LLM/Memory + 刷新会话级快照 | `server/src/engine.rs:293`、`:267`、`:191` |
| 写 | `persist_config()`：临时文件 `rename` 原子写；**单文件 bind mount 下 `rename` 必然 EBUSY** → 回退原地覆写（非原子） | `server/src/app/ws/config.rs:271` |
| HTTP 契约 | 部分更新（缺失=保持 / 显式 `null`=清除）、密钥空串=保留、密钥打码只回 `has_*`、内容指纹 `revision` + 409 乐观并发、`hot/hot_hint/changed_sections` | `server/src/app/ws/config.rs:311`、`:100`、`:212`、`impact.rs` |
| 旁路落盘 | 设备 overlay → `overlays/devices/<id>.toml`；预设 → `presets/<name>.toml`（文件名 sanitize 防穿越 + 读时裁剪越界字段） | `server/src/config/overlay.rs:135`、`:142`、`:57` |
| 无配置文件时 | `PUT /api/config` 直接 **400**（不可持久化）；热句音频落盘同时禁用 | `server/src/app/ws/config.rs:385`；`server/src/engine.rs:126` |
| 客户端 | web 与 mac app 共用 `commonMain` 管理台，走 `/api/config`、`/api/config/meta`、`/api/config/schema`、`/api/config/presets` | `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigFormState.kt` |

**结论：配置的"接口"已经存在且跨端可用；缺的是存储形态。**

---

## 2. 值得做的理由（现状的 5 个真实痛点）

1. **无配置文件就不能保存**：裸跑/首启偶发用内置默认启动时，管理页"保存"返回 400（`ws/config.rs:385`）。
2. **单文件 bind mount 下写配置非原子**：`rename` 不可用只能原地覆写，写一半被杀即配置损坏（`ws/config.rs:276-295`）。
3. **没有历史、没有回滚、没有审计**：保存丢注释，多端并发只有"拒绝"（409），无法对比"改了什么、谁改的、何时改的"。
4. **overlay / preset 是散落文件**：靠文件名 sanitize + 读时裁剪兜底；"应用预设 + 批量改设备 overlay"无法做成一次事务。
5. **配置与记忆各存一处**：备份/迁移要同时处理 `config.toml`、`overlays/`、`presets/`、`memory.db` 四类对象。

DB 化后一次事务搞定，且天然获得历史/回滚/审计。

---

## 3. 🚨 关键约束（不解决必然踩坑）

### 3.1 迁移命名空间**禁止**复用 memory 的 `_migrations`（最高优先级）

事实链：

- 本项目 `server/src/plugins/memory/schema.rs:25` `LATEST_VERSION = 2`，`:45` `migrate()` 读 `SELECT MAX(version) FROM _migrations`，只跑 `(applied+1)..=LATEST_VERSION`。
- 上游 DSH 的 graph-memory 同一张 `_migrations` 编号是 **1..16**（`docs/soul-and-graph-memory-plan.md:349-367`），本项目把上游 m15/m16 两层的表**重编号成了 m1/m2**。
- 而"与 DSH 同库直读"是本仓库已立项的 P3 目标（`schema.rs` 文件头、`soul-and-graph-memory-plan.md:598`）。

后果矩阵：

| 场景 | 结果 |
|---|---|
| 库先由 DSH 建（`MAX(version)=16`），本项目再打开 | `applied=16 > 2` → 循环为空 → 本项目 m1/m2 **永不执行** → `gm_*` 表不存在（P3 目标当场失效，且表现为运行时 SQL 报错） |
| 把配置表编号接在 `_migrations` 的 3/4 | 将来 memory 追加自己的 m3 时，`MAX(version)` 已含配置写的 3 → memory 的 m3 被**跳过**（静默漏迁移：编号语义被两个子系统共享、却各自独立分配） |

**方案**：每个子系统**各用一张迁移表**，编号语义互不干扰：

- `_migrations` → memory（**保持不动**，继续对齐 DSH 编号语义）；
- `_cfg_migrations` → 配置（本设计新增，见 §4）；
- 后续 RAG 索引进库同理。

并加一条护栏测试：迁移表名必须命中白名单常量（防后来者手滑写进 `_migrations`）。

> memory 侧"已有 `_migrations` 1..16 时如何自处"属于独立议题（P3），**不要**和配置迁移混在同一批改动里。

### 3.2 谁 open 数据库：配置 store 必须**自持连接**，不能搭 memory 的便车

`[memory].enabled` **默认 `false`**（`plugins/memory/config.rs:199-205`）→ 走 `NoopMemory` → **记忆库根本不会被打开**（`plugins/memory/store.rs` 只在 `build_memory` 时才 `open`）。
配置是**必需**能力，所以：

- 由 `ConfigStore` 自己 `open`（`busy_timeout` → `journal_mode=WAL` → `foreign_keys=ON` → 自己的迁移），memory 引擎之后 open 同一个文件；WAL 支持多连接一写多读。
- 因此"配置表在记忆库里"在 `[memory].enabled=false` 时**依然成立且可读写**（这是 §7 验收 6 的反向用例）。

**DB 路径解析优先级（唯一权威，需写进 `main.rs` 模块注释与[配置说明](<../AGENTS.md>)；原 `agents.md` §8）**：

1. `XIAOZHI_CONFIG_DB`（env，容器/测试注入）；
2. `--config-db <path>`；
3. 默认 = `[memory].db_path`（**这就是用户要的"与 memory 共用 db"**，生产即 `/data/memory.db`）；
4. 兜底 `/data/xiaozhi.db`。

边界：`[memory].db_path = ":memory:"`（本仓库单测/dev 常见）时不能把配置塞进进程内临时库 → 回退到 1/2/4 的文件路径，并 `tracing::warn!` 一行说明。

并发纪律：`rusqlite` 是同步阻塞 → HTTP 侧一律 `spawn_blocking`（对齐 `app/ws/memory.rs:28` 既有写法）；写事务用 `BEGIN IMMEDIATE` 抢占写锁 + `busy_timeout`（默认 5000ms，与 memory 同值）。

### 3.3 部署形态变了（必须同步改文档与入口脚本）

- SQLite 需要**目录**（`-wal`/`-shm` 与主库同目录）：`-v ./config.toml:/data/config.toml` 这类**单文件挂载彻底不可用**。
  现有 compose 是目录卷 `./server-data:/data`（`docker-compose.yml:54`）→ **不受影响**；但 README/docs 里所有单文件挂载说法、以及 `persist_config` 的 EBUSY 回退分支（`ws/config.rs:276-295`）都要一并退休。
- **备份语义变了**：`cp` 主库会漏 `-wal`。需提供 `POST /api/config/backup`（内部 `VACUUM INTO`）或文档写明 `sqlite3 ... ".backup"`。
- `docker-entrypoint.sh:165-183` 现在做的是「无配置则 `cp config.example.toml` + 配置文件可写自检」→ 要改成「**DB 目录**可写自检 + 首启导入已存在的 `config.toml`」。

### 3.4 人可读 / 可 diff 是真实损失，用导出补齐

TOML 的价值是 git-ops、grep、人工编辑。DB 化后必须有：

- `GET /api/config/export?format=toml|json`（默认打码；`?reveal=1` 需 `expected_token` 鉴权）；
- `POST /api/config/import`（TOML/JSON → 单事务写库；先 `normalize()` + 全量 `validate()`，失败整体回滚）；
- 文档明确：`config.toml` 正式降级为**交换/引导格式**，不再是运行时真源。

### 3.5 密钥与共享库的爆炸半径

`api_key` / `api_secret` 现在是明文 TOML；DB 化后它们与记忆原文同库（P3 后还可能和 DSH 同库）。

- **硬要求**：`SECRET_PATHS` 打码 + `has_*` presence（`ws/config.rs:67`）继续生效，并且 `history` / `export` 默认同样打码（新端点必须复用同一张表，不能漏）。
- 建议：密钥随 JSON blob 走，**不单独建列**（保持单一真源）；文档标注"该库文件视同凭据文件，勿入 git、勿外传"。
- 不做静态加密（引入密钥管理复杂度超出本期，见 §8）。

---

## 4. 存储 Schema

```sql
-- 配置独占的迁移命名空间（§3.1：绝不与 memory 的 _migrations 混用）
CREATE TABLE IF NOT EXISTS _cfg_migrations (
  version    INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);

-- 全局配置：整份 Config 的 JSON blob（等价于今天的 config.toml），唯一一行
-- 为什么是 blob 而不是逐字段 KV 行：现在**全部**语义都作用在"整份文档"上——
-- merge_patch 的部分更新/显式 null 清除、内容指纹 revision、normalize()、
-- SECRET_PATHS 打码、serde 精确类型错误定位。拆 KV 等于把这些重写一遍，
-- 换来的是"可 SQL 查询单字段"这点收益，不值。
CREATE TABLE IF NOT EXISTS cfg_documents (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  doc        TEXT    NOT NULL,   -- 规范化后的 Config JSON
  revision   TEXT    NOT NULL,   -- 与今天同口径的 FNV-1a 内容指纹
  updated_at INTEGER NOT NULL,
  updated_by TEXT                -- 审计：admin-web / mac-app / bootstrap / import
);

-- 历史与回滚（新能力）：保存 / 导入 / 应用预设 / 回滚各留一份
CREATE TABLE IF NOT EXISTS cfg_revisions (
  revision   TEXT PRIMARY KEY,
  doc        TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  source     TEXT NOT NULL,      -- save | import | bootstrap | preset:<name> | rollback
  author     TEXT,
  note       TEXT
);
CREATE INDEX IF NOT EXISTS idx_cfg_revisions_created ON cfg_revisions(created_at);

-- 设备 overlay：取代 overlays/devices/<id>.toml
-- 白名单校验逻辑（validate_overlay / prune_disallowed）**原样复用**，只换存取
CREATE TABLE IF NOT EXISTS cfg_device_overlays (
  device_id  TEXT PRIMARY KEY,
  patch      TEXT NOT NULL,      -- JSON，只含被覆盖字段
  updated_at INTEGER NOT NULL
);

-- 预设：取代 presets/<name>.toml（快照语义与今天一致）
CREATE TABLE IF NOT EXISTS cfg_presets (
  name       TEXT PRIMARY KEY,
  doc        TEXT NOT NULL,      -- 完整 Config JSON
  created_at INTEGER NOT NULL,
  note       TEXT
);
```

可选（P2）：`cfg_audit(revision, changed_paths TEXT, hot TEXT, at INTEGER)` —— 直接落 `impact.rs` 的 `SaveImpact`，管理页即可显示"这次改了哪些字段、多久生效"。

并发模型取舍：

| 方案 | 评价 |
|---|---|
| `Arc<ConfigStore>` 持 `Mutex<Connection>`（单写者） | **推荐**，与 `plugins/memory/store.rs` 同款；跨子系统并发靠 WAL + `busy_timeout` |
| 每请求新建连接 | 简单但每次都要跑迁移检查，且与 memory 争锁更频繁 |
| 连接池多写 | 明确不推荐（SQLite 写锁全局，多写只是把竞争搬到 Rust 侧，见 `soul-and-graph-memory-plan.md:598`） |

---

## 5. 代码落点（改动面清单）

| 文件 | 动作 |
|---|---|
| `server/src/config/store/mod.rs`（新增） | `ConfigStore` trait：`load / save(cas) / revision / history / snapshot / export / import`；`Default` 后端 |
| `server/src/config/store/sqlite.rs`（新增） | 本设计 §4 的实现（含 `_cfg_migrations`、`BEGIN IMMEDIATE`、`VACUUM INTO` 备份） |
| `server/src/config/store/toml.rs`（新增） | 把现有 `persist_config`（`ws/config.rs:271`）**原样搬入**，行为逐字节不变（P0 验收口径） |
| `server/src/main.rs:121/134` | 解析 DB 路径 → `open` store → 空库时导入 TOML → 产出 `Arc<dyn ConfigStore>`；`config_path: Option<String>` 升级为 `ConfigSource::{Sqlite{path}, Toml{path}, Default}` |
| `server/src/engine.rs:105/126/191/267/293` | `config_path` 换成 store；`refresh_from_disk` / `effective_config` / 热句音频目录（改为「DB 同目录 `/hot-audio`」）全部改走 store |
| `server/src/app/ws/config.rs:39/271` | `current_config` / `persist_config` 换成 store 调用（`merge_patch`、`sanitize_patch`、`redact_secrets`、`config_revision` **一行不改**） |
| `server/src/app/ws/config.rs`（新增端点） | `GET /api/config/history`、`POST /api/config/rollback`、`GET /api/config/export`、`POST /api/config/import`、`POST /api/config/backup` |
| `server/src/config/overlay.rs:135/142/149` | 路径函数 → store 读写；`overlay_allowed` / `validate_overlay` / `prune_disallowed` / `merge_overlay` **不动** |
| `server/src/app/ws/mod.rs`（路由表） | 新增 5 条；`GET /api/config/meta` 的 `config_path/persistent` 语义改为 DB 文件 |
| `server/src/plugins/memory/schema.rs` | **不动**（只加"迁移表名白名单"护栏测试，见 §3.1） |
| `docker-entrypoint.sh:165-183`、`docker-compose.yml`、`README.md`、[配置说明](<../AGENTS.md>)、`docs/architecture.md` | 部署/文档同步（§3.3） |
| `client/shared/.../admin/` | **P0/P1 零改动**；P2 加「配置历史 / 回滚」卡片（web 与 mac app 同时获得） |

---

## 6. 分阶段实施（每阶段可独立上线、可回滚）

| 阶段 | 内容 | 回滚方式 |
|---|---|---|
| **P0 抽象层（不改行为）** | `ConfigStore` trait + `Toml` 后端（现有 `persist_config` 原样搬入）；`config_path` → `ConfigSource` | 纯重构，行为不变 |
| **P1 SQLite 后端（opt-in）** | `XIAOZHI_CONFIG_STORE=sqlite`；`_cfg_migrations` + `cfg_documents` + `cfg_revisions`；首启导入 TOML（幂等，`source=bootstrap`）；GET/POST 契约**不变** | 删 env 即回到 TOML |
| **P2 默认切换 + 旁路迁移** | 默认 sqlite；overlays / presets 进表（一次性迁移现有文件）；history / rollback / export / import / backup 端点 + 管理页历史卡 | 保留 TOML 导出可随时倒回 |
| **P3 与 DSH 共库收口** | memory 迁移命名空间按 §3.1 方案改造；DB 级备份/迁移命令与文档定稿 | 单独立项 |

---

## 7. 验收标准

1. **首启导入**：空库 + 存在 `config.toml` → 导入后语义一致（导出 TOML 与原文件 round-trip 等价，忽略注释与键序），日志一行说明来源。
2. **无文件也能持久化**：无 `config.toml`、无 env → 进程正常起（内置默认），且 `PUT /api/config` **成功**（不再 400）；重建容器后配置仍在。
3. **来源可见**：`GET /api/plugins` 的 `config.path/persistent` 指向 DB 文件且 `persistent=true`；旧 400 文案路径已删。
4. **并发语义不回归**：A/B 两端并发保存 → 后者 409 并回传当前 `revision`；密钥空串仍=保留原值；显式 `null` 仍=清除；`hot/hot_hint/changed_sections` 与今天一致。
5. **密钥永不回传**：`GET /api/config`、`/history`、`/export` 默认都只出 `has_*`。
6. **共库双向验证**：`[memory].enabled=true, engine=graph, db_path=<同一文件>` → `cfg_*` 与 `gm_*` 并存、互不删；**`[memory].enabled=false` 时配置仍可读写**（§3.2）。
7. **迁移隔离护栏**：构造 `_migrations` 已有 16 的库 → 配置表仍建立成功，且 memory 不被误判为"迁移编号未定义"（§3.1）。
8. **边界**：`[memory].db_path=":memory:"` → 配置回退文件路径 + 一行 warn，进程正常。
9. **部署文档闭环**：单文件 bind mount 说法从 docs/README 移除；`VACUUM INTO` 备份产物可直接挂回启动。
10. **质量门**：`cargo test`（默认 + `--features sherpa`）0 warning；P2 改客户端时 `./gradlew :apps:h5App:publishWeb` 通过；文件规模仍满足 ≤600 行硬约定（[仓库指引](<../../../AGENTS.md>)，原 `agents.md` §5.10）。

---

## 8. 风险与**明确不做**

| 风险 | 等级 | 缓解 |
|---|---|---|
| 迁移命名空间冲突（§3.1） | **高** | 独立 `_cfg_migrations` + 白名单护栏测试 |
| 部署形态变化（单文件挂载失效） | 中 | 文档 + 入口脚本 + 启动即自检 DB 目录可写 |
| 测试改造面（`config_path` 散布 20+ 处） | 中 | P0 先做无行为变化的抽象层，让测试在 P0 就全部迁完 |
| 与 DSH 双进程争锁 | 低 | WAL + `busy_timeout=5000`（memory 已有同值默认） |
| 密钥进共享库 | 中 | 保持打码契约 + 文档标注，不做静态加密 |

**不做**：逐字段 KV 拆表（§4 取舍）；远端配置中心（违反离线单二进制约束，`p1-features-prd.md` NF-1）；密钥静态加密（本期）；memory 迁移编号改造（P3 独立立项）。

---

## 9. 如果暂时不想动存储（低成本替代）

只解决 1/2 号痛点、不引入 DB 的两个小改：

1. 无配置启动时**自动生成** `config.toml`（`docker-entrypoint.sh:165` 已有先例），取代"PUT 400"；
2. 每次保存前把旧文件备份为 `config.toml.bak.<ts>`。

但**历史/回滚/审计、overlays+presets 事务化、与 memory 共库**这三件，只有 DB 化能给。

---

这些 schema、阶段与验收是设计目标，不是已支持的运行契约；当前配置后端仍为文件 / 内置默认，使用方式以源码、样例与部署说明为准。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
