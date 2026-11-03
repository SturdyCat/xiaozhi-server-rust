# 天气预报技能（weather_query / weather.forecast）

本文档说明新增的「天气预报」意图与底层能力，以及让本地 NLU **真正识别** 天气意图所需的最小重训步骤。

## 1. 新增内容

### 技能意图 `weather_query`
- 种子文件：`server/src/plugins/nlu/defaults.json`（仅首次空库播种；后续以 SQLite 保存态为准）。
- 槽位：`city`（城市，可选）、`range`（时段，可选）。
- 模板（均为单槽或字面量分隔，满足 `catalog::validate_template`「两个开放槽不能直接相邻」约束）：
  `{city}天气` / `{range}天气` / `{city}的天气` / `{range}的天气` / `查天气` / `天气怎么样` / `查一下天气` / `今天天气` / `明天天气` / `后天天气`。
- 动作：两步 plan —— 先 `weather.forecast`，再把其产出的 `{{steps.0.speech}}` 交给 `replies.say` 播报。

### 能力 `weather.forecast`
- 声明：`server/src/skills/plan.rs` 的 `capabilities()` 白名单（execution 标记为 `"network"`，仅元数据）。
- 校验：`validate_step` 允许 `args` 仅含 `city` / `range` / `province`（均可选）。
- 调度：`server/src/skills/api.rs` 在 `execute` 开头单独特判该 api，转交 `server/src/skills/weather_ops.rs`。
- 引用放行：`server/src/skills/results.rs` 已把 `weather.forecast` 加入「可暴露 speech 的前序步骤」白名单，使 `{{steps.0.speech}}` 合法。

### 网络与定位（不放在 skills/ 内）
- `server/src/plugins/weather/{mod,parse,tests}.rs`：**唯一**发生天气 HTTP 的地方（`mod.rs` 持有 `reqwest::blocking::Client` 与按站点码的 TTL 缓存，`parse.rs` 为宽松解析纯函数）；纯 Rust 复刻国家气象中心公开接口（参考 `valenovo/china-weather-agent-skill`，但不 vendoring 任何 python 脚本——镜像 `debian:bookworm-slim` 无 python3）。
- `server/src/plugins/geo/{mod,tests}.rs`：启动期解析服务器**出口 IP** 作为默认定位。仅当为**国内** IP（`myip.ipip.net`）才写入，**国外/翻墙出口直接放弃（不触发兜底）**；仅当主源请求/解析失败或城市为空才兜底 `whois.pconline.com.cn`（国内 IP 库）。
- 出口信息展示：`GET /api/server/geo`（`server/src/gateway/ws/registry.rs` 的 `server_geo`）返回最近一次成功解析的出口 IP 与地理信息（**含境外**，`domestic=false` 仅展示不采用）及未定位原因；管理台「概览」页的「服务器出口」卡消费该端点（`OpsState.GeoState` + `OpsOverviewPage.ServerGeoCard`）。
- `server/src/skills/AGENTS.md` 已注明：`weather.forecast` 是本目录唯一允许联网的能力，网络实际发生在 `plugins/weather`。

### 配置
- `server/src/config.rs` 新增 `#[serde(default)] pub weather: WeatherConfig`，并提供 `pub use crate::plugins::weather::WeatherConfig`。
- `server/config.example.toml` 新增 `[weather]` 段（`engine` / `source` / `timeout_secs` / `cache_secs`，默认值即可工作）。
- 装配：`server/src/runtime/mod.rs` 的 `Engines::assemble` 内 `set_global` 天气服务与地理定位锁；`server/src/main.rs` 启动后 `tokio::spawn(crate::plugins::geo::resolve_location())`。

## 2. 关键提醒：本地 NLU 是纯 BERT 模型，新增意图必须重训

本仓库 NLU 是**纯 TinyBERT 模型识别**（`bert_enabled=true` 时），**没有规则匹配、没有 LLM 兜底、没有启发式回退**。模板（`defaults.json`）只用于在模型**已经接受**同一意图后抽取槽位，**不能**靠模板去选中意图。

因此：要让本地模型真正触发 `weather_query`，必须：

1. **把 `weather_query` 加入训练标签与样本**，重训 TinyBERT。
2. 在 `[nlu].bert_label_map` 里加 `"weather_query": "weather_query"`（或把模型输出的标签映射到该名），保证分类结果能落到技能目录。
3. 训练/候选/验收走既有链路：`tools/nlu-trainer/`（`train.py` 等）+ `server/src/gateway/ws/nlu_build.rs` 的 `bert_label_map` 与候选测试。具体训练脚本与数据格式以 `tools/nlu-trainer/AGENTS.md` 与 `docs/designs/tinybert-nlu-train-pipeline.md` 为准。

### 建议加入训练数据的 15 条中文样本（覆盖实时/当前/7天/城市/默认）
- 北京今天天气怎么样
- 查一下上海未来一周的天气预报
- 广州近7天天气
- 深圳的天气
- 现在外面冷不冷（默认地区实时）
- 明天会下雨吗（默认地区 + 明天）
- 杭州后天天气
- 成都7天预报
- 实时天气（默认地区实时）
- 当前气温（默认地区当前）
- 南京天气
- 武汉未来7天
- 西安今天多少度
- 重庆的天气预报
- 查天气（默认地区）

> 注意：「城市 + 时段」组合（如「北京今天天气」）由 `weather.forecast` 处理器在运行时**二次切分** `city` 串中的时段词得到，不需要双槽模板，也不需要在训练语料里穷举所有组合。

## 3. 出口 IP 默认定位
- 服务器启动后异步解析出口 IP，仅**国内** IP 写入默认地区（经 `myip.ipip.net`；**国外出口直接放弃、不兜底**；主源请求/解析失败或城市为空才兜底 `whois.pconline.com.cn`）。
- 任何网络/解析错误都只 `tracing::warn!` 后返回，**绝不阻塞启动、绝不 panic**。
- 未定位到默认地区时，天气必须显式给城市，否则返回友好提示而非报错。

## 4. 数据来源合规
- 天气数据来自**国家气象中心（NMC）公开接口**，仅供辅助查询，**不构成专业气象决策**。
- 仅作个人/家用语音助手级查询使用，请遵守数据源服务条款与合理频率（本实现已内置按站点码 TTL 缓存，默认 600s）。
