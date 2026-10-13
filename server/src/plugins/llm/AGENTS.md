# LLM 请求与护栏维护

当前[OpenAI 实现](<openai.rs>)使用兼容 **Responses API**，不是 chat/completions；没有生产 mock。连接失败先查 api_base/api_key/model。

- [LlmProvider](<mod.rs>)收 TurnPrompt；instructions 是稳定前缀，记忆块按 inject_position 进 input，不能把动态记忆一股脑拼进稳定人格。
- 失败按[具名类别](<failure.rs>)路由，不解析错误文案。超时分建连/请求兜底/流空闲，重试由[装饰器](<retry.rs>)执行，有次数与总预算。
- 已吐文本后不重试，避免重复播报；取消不重试，空回复不是正常成功。失败经会话兜底而非静默。
- usage 是 token 口径，不是金额；估算与真实上游用量区分，CJK 不能照搬四字符一 token。
- 新 provider 必须实现 TurnPrompt 和流式取消契约，并覆盖 SSE、失败分类、空回复、重试边界与用量测试。

回归：`failure/tests.rs`、`openai/tests.rs`、`retry/tests.rs`。人格与提示词组装见[soul](<../soul/AGENTS.md>)、[prompt](<../prompt/AGENTS.md>)。

---

## 护栏与提示词设计（历史记录）

> **历史设计 / 实施记录**：由原《插件架构统一》长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/plugin-architecture-unification.md>)定位；当前行为先读本文件上半部分、源码与测试。
> 下文含未落地路由/折叠等提案，不能据此新增无效配置字段。

## 4. 借鉴 DSH 的 LLM 插件：只借"护栏"，不借"编排"

> 本节所有 DSH 结论均来自 asar 内该包 `README.md` / `lib/*.js` 原文（引号内为原文），并对齐 §1.4 里本仓库的实测缺口。

### 4.0 先纠正一个容易想当然的前提：DSH **没有** provider 故障转移

这一点必须先说清楚，否则会"照抄一个不存在的东西"。三条独立证据：

1. `dsh-llm/README.md:74`：*"This service never re-runs a request: retrying is the job of `dsh-llm-retry` at the agent's failed-step extension point."*
2. `dsh-llm-retry/lib/index.js` 的 `recover()` 命中策略后只产出 `{kind:'retry'}`，让循环**在同一 provider/model 上重跑该 step**；文件内没有任何"换 provider/model"的分支。
3. 对 290 个 `@deepseek-ai/*` 包全量检索 `fallback`：命中的全是 `defaultContextWindow` 容量回退、图片 base64 inline 回退、标题兜底，**没有一例是 provider/model 故障转移**。

**DSH 的模型侧韧性 = 同路由重试 + 压力折叠**，不是"多 provider 降级"。对我们的意义：**`[llm.fallback]` 若要做，是我们自己的设计，不能记在"借鉴 DSH"账上**。建议默认**不做**（语音一次回复只打一次请求，多打一份 = 多一份 TTFA 与费用）；确需降级时也只对**可重试类错误**（连接失败 / 429 / 5xx）生效，绝不"永久失败也换"。

**顺带一条不变量（可作 §3.2 的外部依据）**：adapter 注册是**原子 + 单所有者 + 注册即固化策略**：

- `ctx.llm.registerAdapter(providers, adapter)` 是**全有或全无**的原子注册：任一路由已被占用 → 抛 `LlmError{code:"DUPLICATE_ADAPTER"}`（`dsh-llm/lib/index.js:1863`，消息 `an adapter for provider "..." is already registered`）；
- 注册时**捕获该路由的 retryPolicy**（`lib/index.js:1867`：`const retryPolicy = adapter.providerRetryPolicy(provider) ?? resolveRetryPolicy(...)`）；
- README 不变量原文：*"Registry mutations are atomic — route and directory registration validates the whole candidate set before anything moves, so a refused change leaves the previous state serving."*（**拒绝变更后旧状态继续服务**）。

**适合我们**：我们的 `[llm]` 是单后端，没有"路由冲突"问题，但**"注册即固化策略 + 拒绝变更不破坏当前状态"**这条不变量，正是 §3.2 里 `PluginHost::refresh_from_disk` 的语义（重建失败保留旧实例）。可直接引用作为该设计的外部依据。


### 4.1 命名化失败分类 + 有界重试（`dsh-llm-retry`）

**DSH 的做法**：重试策略**不在重试插件里**，而在每个 provider profile 上（`retryPolicy`），`dsh-llm-retry` 自称 *"It is the executor: the retry policy itself lives on each provider adapter's configuration, and this package has no configuration of its own."*

- 失败分类是**具名枚举**：`EMPTY_RESPONSE` / `RATE_LIMIT` / `SERVER` / `TIMEOUT` / `TRANSPORT`；
- **normal 模式默认**：上述 5 类各 5 次重试，指数退避 **500 ms → 10 s，10% 抖动**；
- **always 模式**：无次数上限，直到成功/取消/dispose（原文：*"always mode continues until success, cancellation, or disposal"*）；
- **`Retry-After` 优先**：*"A valid `Retry-After` from the provider replaces local backoff when it fits the policy bounds."*
- 取消语义：*"Cancellation or plugin disposal aborts active backoff … and makes a callback captured before disposal fail closed."*
- 成本与可见性：*"Each retry is another billed provider request"*、*"Nothing here is model-visible: no retry event, delay, provider error, or failed partial output reaches the model."*

**精确默认值（源码逐字，已核对 `dsh-llm/lib/types/retry-policy.js:12-21`）**：`DEFAULT_MAX_RETRIES=5`、`DEFAULT_INITIAL_DELAY_MS=500`、`DEFAULT_MAX_DELAY_MS=10_000`、`DEFAULT_JITTER_RATIO=0.1`、可重试码 = `[EMPTY_RESPONSE, RATE_LIMIT, SERVER, TIMEOUT, TRANSPORT]`。退避公式（`dsh-llm-retry/lib/index.js:44`）：`delay = min( min(initial × 2^(retry−1), max) × (1 − jitter + 2·jitter·rand()), max )`。校验是 fail-loud 的：未知键报错、`retryableCodes` 不得为空/重复、`initial ≤ max`。

**分类纪律：按 code 路由，绝不解析 message**（`dsh-llm/lib/types/error.js` 头部原文 *"route on this, never by parsing `message`"*）。码表：`AUTH(401/403)` / `QUOTA` / `RATE_LIMIT` / `CONTEXT_WINDOW_EXCEEDED` / `INVALID_REQUEST` / `SERVER` / `HTTP_<status>` / `TRANSPORT`（传输失败）/ `ABORTED`（取消）/ `TIMEOUT`（**stream-idle 到期**）/ `EMPTY_RESPONSE`（退化完成＝空回复，必须当失败）。

**超时与重试是两件事**：`retryPolicy` **不含超时字段**；模型侧超时在 adapter 上（`streamIdleTimeoutMs`，DeepSeek 默认 300,000 ms，语义是"单次流读取的最大空闲时间"）。`Retry-After` 若 ≤ `maxDelayMs` 则优先于本地退避；**normal 模式下超过上界就放弃重试**。

**我们的缺口（实测三处）**：

1. `openai.rs:82-85` 的 client 只设了 UA，**没有 `.timeout()` / `.connect_timeout()`** → 上游挂起即永久等待；
2. `openai.rs:163-166` 把状态码与响应体**拼成一个字符串** `bail!("LLM API 返回错误 {status}: {body}")` → 调用方**无法按状态分类**，重试与降级都无从谈起；
3. **没有"空回复"判定**（`EMPTY_RESPONSE`）——退化完成会被当成成功，用户侧表现就是"没回应"。

**落法**：把重试做成 trait **之上的装饰器**（照抄 DSH「服务不重试、执行器重试」的分层），`LlmProvider` 保持单次尝试语义：

```toml
[llm]                        # 超时属传输层，不属重试策略（对齐 DSH 的 streamIdleTimeoutMs）
connect_timeout_ms     = 3000
stream_idle_timeout_ms = 30000

[llm.retry]                  # 只提供 normal；**不提供 always**（语音不可无限重试）
max_retries       = 5
retryable         = ["rate_limit","server","timeout","transport","empty_response"]
initial_delay_ms  = 500
max_delay_ms      = 10000
jitter_ratio      = 0.1
honor_retry_after = true
```

Rust 侧新增 `LlmFailure` 枚举（`RateLimit / Server / Timeout / Transport / EmptyResponse / Auth / Quota / ContextWindowExceeded / InvalidRequest / Http(u16)`）；`openai.rs` 只负责「HTTP/传输 → code」的映射，重试循环放在装饰器里。**两条语音特有约束**：① 重试总预算要有上界（建议 ≈8 s，超了就放弃并走兜底话术）；② **已下发音频后不再重试**（见下）。

实现要点（**与 DSH 的关键差异，对我们更有利**）：DSH 明确承认 *"Provider HTTP status is unavailable — pi-ai error events do not expose a stable HTTP status across providers"*，所以它只能按抽象码分类；**我们用 `reqwest` 直接拿到 status**，因此分类可以直接建立在 429/5xx/连接错误上，并把 status 一起写进日志与 `/api/plugins` 的 `last_error`。

⚠️ **流式重试的边界（必须写进实现注释）**：一旦已经有 chunk 下发到设备，**不能再重试**（会重复播报）。规则：**首个 chunk 之前**的重试才允许；首 chunk 之后的失败只能向设备发 `tts:stop`/错误提示。这与 DSH 的 *"a raw stream cannot separate already-emitted chunks durably"* 是同一个约束。

### 4.2 路由级模型元数据与"写时校验"（`dsh-llm-pi-ai`）

**DSH 的做法**：一份 `providers` 字典描述所有端点，每条 profile 含 `apiKeyEnv` / `baseURL` / `api` / `models[]` / `retryPolicy`；每个 model 可声明 `contextWindow`（README 示例为 `200000`、`262144`）。校验语义尤其值得学：

- *"A route pi-ai does not ship needs `api`, `baseURL`, and a non-empty `models` list; an unserviceable profile is **refused where it is written, naming the route and model**."* → **写时就拒绝，并指名道姓**（而不是启动后才炸）。
- *"Config updates strictly validate **changed** providers. Initial loading retains stored catalog failures as editable provider diagnostics; unchanged failed providers do not block."* → **只校验被改动的条目**；历史遗留的错误条目不该阻塞启动，但要能被 UI 展示与编辑。
- `apiKeyEnv`：*"a credential reference resolved per request through the harness credential seam"* → 密钥是引用不是值。
- 已知坑（原文）：*"`headers` can carry a credential **the redactor never sees**"* → 我们的打码逻辑必须把自由 `headers` 字段视为可能含密钥。

**我们的缺口**：`[llm]` 是扁平的 `api_base/api_key/model/system_prompt/max_history/temperature/stream`（`llm/mod.rs:13-24` + Default impl），**没有 model 元数据、没有 context window**，因此 §4.3 的"占用率"与记忆注入预算都无处标定。

**落法**：见 §3.3 的 `[llm.<id>]` 段 + `[llm.routing]`；`/api/config` 的 POST 校验规则 = "只校验被改动的 profile，失败要指名 route + model"（对齐 DSH 语义），错误直接返回前端做行内提示。

### 4.3 Token 计量与"上下文压力"分解（`dsh-token-meter`）

**DSH 的做法**（`ctx.tokenMeter` 两个操作：`measure(session, requestHeader?)` / `estimateMessage(message)`）：

- 会话级读数：`totalTokens`（请求+响应压力）与 `surfaceTokens`（仅模型可见面）；
- 用量分解：`uncachedInputTokens` / `outputTokens` / `cacheReadTokens` / `cacheWriteTokens`（**缓存读写分开**）；
- 压力：`contextPressure{ pressureTokens, projectedTokens, contextWindow }`，其中 `contextWindow` 来自 *"`ctx.llm.resolveModelInfo().context`"* —— **容量属于 adapter/route，不属于计量器**；
- 组合分解：`contextBreakdown{ systemTokens, toolsTokens, messageTokens }`；
- 诚实声明局限：*"The `contextBreakdown` figures are estimates … **CJK text and JSON schemas underprice badly at four characters per token**"*，且 *"Occupancy is a reference figure, not a billing record"*。

**我们的缺口**：完全没有用量观测。这有两个后果：① 成本不可见；② **无法回答"记忆/灵魂到底让 prompt 长了多少"**——而这正是记忆功能上线后第一个会被问的问题。

**落法**：四桶命名与口径**逐条对齐** DSH（这样将来对账/对标不会各说各话）：

```toml
[llm.usage]
enabled      = true
log_per_turn = true          # 每轮 debug 日志：input/output/cache_read/tools/system/recall + ttfa_ms
```

- 每会话持一个 `TokenMeter`（随 `session.rs` 生命周期），四桶：`uncached_input` / `output` / `cache_read` / `cache_write`；**一次重试算另一次计费尝试**（对应 DSH 的 *"Each retry is another billed provider request"*）；
- **压力口径 = `input + cache_read + cache_write`（不含 output）**——与 DSH 的 `pressureFrom` 一致，且必须与 §4.5 折叠触发的口径**用同一函数**，否则"UI 说没满、折叠却触发了"；
- 只读端点 `GET /api/session/{id}/usage`（按会话，不按全局；DSH 也是 session 级 fold）+ `GET /api/llm/usage`（进程累计，便于运维）；
- **不做金额换算**：DSH 全包**没有**任何单价/金额字段（只有图片视觉 token 计价），README 亦声明 *"Occupancy is a reference figure, not a billing record"*。各种 OpenAI 兼容网关单价不同，硬编码会误导用户——**只报 token**；
- **中文启发式必须自校准**：不要照抄 `CHARS_PER_TOKEN = 4`（DSH 原文自陈 CJK 严重低估）。按字符区间分段：CJK 汉字/CJK 标点按 **≈1 token/字**，ASCII 按 **≈4 字符/token**，并在 UI 标注"估算值"；
- 记忆注入预算改成**以 token 为单位**（`recall_max_tokens` + `context_share`），与 `[llm.context].window_tokens` 联动——这直接改进 `soul-and-graph-memory-plan.md` §5.6 的 `recall_max_chars`。

### 4.4 有序 prompt 段：把"灵魂/记忆"做成并列 section（`dsh-system-prompt` + `dsh-persona`）

**DSH 的做法**：

- `ctx.systemPrompt` 是**注册表**，插件的贡献按 `order` **升序拼接**，且 *"equal orders use code-unit name order"*（**同级有确定性 tie-break**，避免顺序随机）；
- 实测内置 order：固定开场白 **−1000**、`personaPrefix` **0**、`personaSuffix` **10200**（`persona` 与"部署级人格"是**同一套 section 机制**里的条目，不是特例）；
- 变量：`{{name}}` 每次组装时求值，*"scoped variables shadow a same-named global for that agent"*；
- `interpolate: false` 可让某段**保持字面**（防 `{{}}` 被吃掉）——对"记忆原文里出现花括号"的场景是必需的保险；
- **Prefix 稳定性 = prompt cache 命中率**：*"Prefix-stable while identity, persona, variables, section text, and order render identically"*、*"reordering changes cache shape → may invalidate reuse"*、*"Schema tokens repeat on every request"*。

**DSH 的实际段号表是"稀疏编号"**（`dsh-system-prompt/lib/index.js:10`，可直接借用这个口径）：`HARNESS_IDENTITY -1000` / `DEPLOYMENT_PERSONA_PREFIX 0` / `PLAN_POLICY 500` / 各工具 `1000–3100` / `TOOLS_SDK 5000` / `DELIVERABLE_FILE_REFERENCES 9000` / `STRUCTURED_OUTPUT 9900` / `HARNESS_SOURCE 10000` / `WEB_SURFACE 10100` / `DEPLOYMENT_PERSONA_SUFFIX 10200`（运行时事实另有 `CONTEXT_ORDERS`：sandbox 110 / approval 115 / subagent 120）。**用稀疏数字（间隔 100~500）而不是连续序号**，是为了以后往任意位置插一段而不必重编号——我们的 `[soul]`/`[memory]`/技能/运行时事实也应照此留空档。

**插值的纪律是"宁可吵闹地失败，也不要畸形的 prompt"**：DSH 对未知引用、已注册但值为 `undefined`、畸形 `{{a}b}}` **一律抛错**（原文 *"a malformed prompt is worse than a loud failure"*）；孤立 `{{` 当普通文本；**替换结果不再二次扫描**；空段丢弃、`\n\n` 连接；要保留字面花括号必须 `interpolate: false`。→ 我们的 `assemble()` 同样应在**启动期**校验未知变量（而不是渲染出 `{{memory}}` 塞给模型）；记忆原文里可能出现花括号，因此记忆段应显式 `interpolate: false`。

**落法（也是对本项目既有设计的一处修正）**：

`[soul]` / `[memory]` 都做成 `systemPrompt` 注册表的**段提供者**（`order` 从描述符里声明），而非往一个字符串里拼。**顺序建议**：

```
order 0     : 身份与人格（[soul] 前缀，**稳定**，只随配置变化）
order 100   : 能力/风格约束（[soul] 后缀：句长、口语化、拒答边界）
order 900   : 工具说明（若有）
order 1000  : 历史（append-only 前缀，缓存友好）
order 1100  : 记忆召回（**易变**，放在历史之后 → 保住前面整段的 prefix cache）
order 1200  : 当前用户输入（由 input[] 承载）
```

⚠️ **权衡要说清楚**：`soul-and-graph-memory-plan.md` §4.3 原设计把记忆放在历史**之前**（该节已加修订批注）。改为之后有得有失：**得** = 前缀缓存命中（`max_history` 越大越值钱，DeepSeek/OpenAI 的 prefix cache 都按前缀命中计费）；**失** = 召回内容离用户问题更远、recency 稍弱。因此给一个显式开关（`[memory].inject_position = "after_history" | "before_history"`，默认 `after_history`），并在文档里写明：`max_history` 调大时才考虑 `before_history` 的收益反转。**不要靠猜，用 §4.3 的 `cache_read` 读数验证。**

⚠️ **别高估 prefix cache 的收益（本项目特有）**：`session.rs:239-241,568-570` 的历史截断是**从头丢弃**（`history.remove(0)`），一旦发生截断，整个历史前缀都移位 → 缓存失效。也就是说 prefix 稳定性只对"还没触发截断的对话"有效。**结论：先把 §4.1 的重试与 §4.3 的计量做出来，再谈段序优化**；段序按上面给的默认值即可，不要提前做缓存微调。

### 4.5 折叠（compaction）：**先别做，用观测决定**（对调研结论的一处修正）

**DSH 机制**（`dsh-compaction-basic`）：
- 触发公式（`lib/index.js:128-133`）：`messageBudget = W − O`；`threshold = floor(min(W × thresholdRatio, W − O − headroom))`；默认 `thresholdRatio=0.8`、`retainRatio=0.16`、`headroomTokens=65536`；
- 压力口径 = **输入 + cache 读写（不含输出）**，与 `dsh-token-meter` 的 `pressureFrom` 一致；
- 模型看到的东西：选中区间被**一条 user-role 消息**替换 = preamble + `<compacted-summary>` 摘要 `</compacted-summary>`，后接**原样保留的近期 tail**；preamble 明确要求"当作既定背景、不要复述、不要确认这个 checkpoint"；
- 摘要请求复用 `system node 0 + 上次 tools + 被 shadow 的消息（逐字节）`，把指令作为**最后一条 user 消息** → 目的是**吃前缀缓存**；
- 选区从"第一个非 `system/message` 节点"开始，必须 tool-pairing 平衡；**`system` 段永不被折叠**。

**判断（这里与调研报告有分歧，给出取舍）**：调研把折叠列为"当前最大功能空缺"，我认为**对本项目应降级为条件项**：

- 语音历史本来就短（`max_history` 默认 10 条 ≈ 5 轮），而**记忆召回（P4）已经覆盖"跨会话事实"**这一类需求；
- 折叠的真实收益（连续对话超过 ~5 轮仍记得细节）在语音场景出现频率低，成本是"一次额外 LLM 调用 + 一整套复杂度"；
- 真正该修的是**硬截断这个语义**（见 `soul-and-graph-memory-plan.md` §1.2；本仓库证据 `session.rs:239-241,568-570`：按条目从最旧开始丢），而它有更便宜的解法：把 `max_history` 换成**按 token 预算保留最近 N 轮原文**。

**落法**：P4 先做「记忆召回 + `[llm.context]` 预算化截断 + 用量观测」；**只有观测证明截断造成实际遗忘**（`contextBreakdown.messageTokens` 频繁触顶、且有用户抱怨"忘事"）才上折叠，最小实现 = `[llm.context].summarize=true` + 折叠成一条 checkpoint 消息 + 复用同一端点。**不要**引入 DSH 的 pruner / spill / 事务日志锁整套。中文估算必须用 `chars/1.6` 而非 DSH 的 `chars/4`（后者对中文低估约 2.5×）。

### 4.6 工具管线：若将来做 MCP 闭环，借两条纪律（`dsh-tools`）

- 固定管线：`pre-execute`(allow/deny/ask) → **单调 guard** → `execute` → `post-execute` → `finalizeContent` → 只读 `result`；
- **`pre-execute` 故意不能修改 `exec.arguments`**——原文理由：否则 *"logged and rendered args would desync from what ran"*（审计一致性优先于灵活）；
- **guard 单调不可翻转**：拒绝是硬事实，后续 listener 不能改回允许；
- **拒绝 = 工具结果，不是异常**：产出 `Error: <reason>` + `isError:true` 回填给模型，让模型自己说"这个我做不到"，**不中断 WebSocket**；
- `ask` 需要审批通道，无通道时**降级为 deny** → 我们**不实现 `ask`**（ESP32 无 UI 可审）。

落法：`ToolSpec` 增加 `guard: fn(&ToolCall) -> Option<String>`（只读参数、同步、单调），拒绝回填 `function_call_output` 的 `Error:`。这也是 `gm_search` 上线时的安全底座。

### 4.7 LLM 卡的信息架构（`dsh-client-ui-settings-models` 的三条原则）

1. **高频字段在卡上，低频/高级字段折叠或干脆不上 UI**。DSH 明确把 retry/timeout 排除在设置页之外（README：*"Retry policy, timeouts ... remain in `cordis.patch.yml`"*）——**这条对我们直接适用**：§4.1/§4.5 新增的 `[llm.retry]`/`[llm.context]` 字段**不要平铺到卡片**，放"高级"折叠区（默认收起）；主卡只留 `api_base` / `api_key` / `model` / `stream`。
2. **密钥只写不读**。DSH 的 provider 行主字段只有一个 API key 输入，write-only，页面**从不询问环境变量名**，凭据状态用绿点/红点表示。我们的现状相反（`GET /api/config` 明文回显 `api_key`，`ConfigFormState.kt:437/533` 明文往返）→ 配合 §3.4 的打码 + "留空即不改"语义改造。
3. **写前校验 + 字段级错误**。DSH 要求 key **全为可打印 ASCII**（`[\x21-\x7E]`，因为它最终要进 HTTP header）、拒绝 `NAME=value` 粘贴形式、容量非法即拒、写入带 `revision` 且冲突返回 `settings/conflict`。→ 我们的 `POST /api/config` 应对 `api_base`（http(s) URL）、`model`（非空）、`api_key`（可打印 ASCII）做写前校验并给字段级提示（`ConfigFormState` 已有 `dirty`/`vif` 机制，改动面小）。

**附带可借**：DSH 的「Fetch available models」会**真去问端点**再让用户选择，而不是手打 model 字符串 → 我们的「测试连接」动作（§3.4 的 action 模型）应同时返回**可用模型列表**，卡片上用官方 AlertDialog 下拉选择。

### 4.8 明确**不借鉴**（语音单轮场景不划算）

| DSH 包 | 为什么不用 |
|---|---|
| `dsh-agent-loop` / `dsh-subagent*` / `dsh-workflow*` / `dsh-tool-ralph` / `dsh-experimental-agent-team*` | 语音是一问一答 + 可打断；多步 agent 循环会把 TTFA 变成多跳模型调用，且设备端没有"计划/审批"交互面 |
| `dsh-ptc-runtime*`（`run_code` + TS/Python SDK 生成） | 依赖 Node/Python 运行时；其自陈 *"intermediate values are execution-local and unbounded by bytes — may exhaust process or worker memory"*；我们也不该有脚本执行面 |
| `dsh-tool-*`（bash / fs / pwsh / web / workflow…） | 语音终端**不应该**有任意工具执行面（安全与延迟双重理由）。若要做，只暴露只读白名单 + §4.6 的单调 guard，并沿用已有 `ToolSpec` |
| 交互式审批（`ask` / `dsh-user-approval` / `sandbox_permissions` 提权） | ESP32 无 UI 可审；DSH 自己也定义「无审批通道 → 降级 deny」 |
| `dsh-web-search-deepseek` / `dsh-tool-web` | 拉长 TTFA；要联网应是**显式开关的独立能力**，不是默认注入 |
| `always` 重试模式 | 认证/配额/非法请求会被无限重试且持续计费；语音必须有限预算 |
| 图片/文件/视觉预算体系（Files API、`IMAGE_OFFLOAD_REQUIRED`、`*-image-offload`） | 我们上行只有 Opus 音频，无图片输入；整套投影/上传/配额是纯复杂度 |
| `dsh-deepseek-llm-api-extensions` 字段注册表 | 我们只有一处请求构造点，`serde_json::json!` 直接写即可 |
| `dsh-agent-instructions`（AGENTS.md 链加载 / digest / touch 刷新） | 依赖文件系统与工作区概念，我们没有。**只借它的"字节预算 + 更具体优先"裁剪顺序**用于记忆段限长（broad 先丢、最具体才截断） |
| `dsh-session-title-llm` 等"每轮额外 LLM 调用"插件 | 每轮多一次计费调用；语音端只保留**抽取**这一处必要调用（且放后台，见 soul 文档 §5.2） |
| session-projections / checkpoint / 回放式确定性 fold | 我们不持久化会话日志（会话即连接），引入 fold 引擎复杂度收益为零 |
| pi-ai 多协议目录 / OAuth 登录 / 跨进程刷新锁 | 无 Node、无凭据存储组件；我们只需一个 OpenAI 兼容端点 + `api_key` |
| MCP 客户端的完整实现（`dsh-mcp-*`） | 设备端虽有 `mcp` 消息类型预留，但服务端做完整 MCP 客户端会放大延迟与安全面；本期不做（只做 §4.6 的 guard 底座） |

### 4.9 我们比 DSH 更容易做对的三件事（别浪费）

1. **有真实 HTTP status**：重试分类可建立在 status 上（§4.1），比 DSH 的抽象码更准；
2. **单进程、单入口**：不需要 DSH 的 session log / 投影 / 复盘机制来保证"重试后历史一致"——我们只需保证 `history` 是**不可变事实源**、模型可见面**每次重新组装**（这点 `session.rs` 已经是这么做的）；
3. **中文优先**：token 启发式必须为 CJK 校准（§4.3），而 DSH 的默认启发式在中文上是已知低估的。

---
