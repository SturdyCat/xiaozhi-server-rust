package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.View

// ============================================================
// 「灵魂」「记忆」两个配置卡（从 ConfigCards.kt 拆出，见 AGENTS.md §5.10）
// ============================================================
// 设计依据：docs/soul-and-graph-memory-plan.md §7（tab 布局 + 交互语义）。
// 表单字段状态在 `ConfigContextState`（`form.sm`）；JSON 往返见其 `putJson`/`fill`。
// ⚠️ 新增 UI 一律复用 AdminTheme/AdminFormControls 里的官方组件组合，禁止自造样式。
// 文案口径：字段名用中文（key 不上界面），解释性长段落收进 ⓘ 弹层（见 [fieldLabel]）。
// ============================================================

/**
 * 灵魂（`[soul]` 人格档案）配置卡。
 *
 * `enabled = false` 时完全退回 `[llm].system_prompt` 的原文（逐字节一致），
 * 因此关闭态的提示必须写清这一点，用户才敢打开 / 敢关掉。
 *
 * 「内置默认灵魂」（`preset`）的交互三件套：
 * 1. **载入预设内容到表单** —— 把服务端合并出的完整档案写回表单，于是默认人格变成
 *    可逐条修改的显式值（此前只存在于运行时合并结果里）；
 * 2. **清空人格字段** —— 回到"只用预设"的状态（留空 = 运行时由预设补）；
 * 3. **预览最终提示词** —— 把实际发出去的 instructions 与"哪些字段来自预设/被你覆盖"
 *    摊开；顺带当保存前校验器（preset 拼错会当场报错）。
 */
fun ViewContainer<*, *>.soulConfigCard(form: ConfigFormState, ctx: Pager) {
    val sm = form.sm
    groupedCard("人格档案") {
        switchRow(
            "启用人格档案", sm.soulEnabled == "true",
            help = "关闭时系统提示词 = LLM 卡的系统提示词（原文，可随时回滚）。" +
                "打开后由「平台约束 + 人格 + 表达风格 + 系统提示词」按序组成；保存后**新会话生效**。",
        ) {
            form.toggleStringBool(sm::soulEnabled)
        }
        // ===== 未启用：一键用内置默认人格开启（开箱可用）=====
        vif({ sm.soulEnabled != "true" }) {
            textNote("内置了一份默认人格「小智」：打开就能用，之后每个字段都可以改，也能整份换成自己的。", micro = true)
            actionRow {
                secondaryButton("用内置默认人格启用") {
                    sm.soulPreset = "xiaozhi"
                    sm.soulEnabled = "true"
                    form.markDirty()
                    sm.soulPresetMsg =
                        "已启用内置默认人格：你没填的字段由预设补，填了的字段覆盖它。" +
                            "点「预览最终提示词」看实际发出去的内容。"
                }
            }
            vif({ sm.soulPresetMsg.isNotEmpty() }) {
                textNote(sm.soulPresetMsg, micro = true)
            }
        }
        vif({ sm.soulEnabled == "true" }) {
            dividerH()
            // ===== 内置预设（默认灵魂）=====
            dropdownField(
                label = "内置人格预设",
                currentLabel = {
                    sm.soulPresetOptions.firstOrNull { it.first == sm.soulPreset }?.second ?: sm.soulPreset
                },
                options = { sm.soulPresetOptions },
                selectedId = { sm.soulPreset },
                isOpen = { form.isDropdownOpen("soul_preset") },
                onToggle = { form.toggleDropdown("soul_preset") },
                onSelect = {
                    sm.soulPreset = it
                    form.markDirty()
                    form.openDropdown = ""
                },
                help = "内置默认灵魂：**留空**的字段由预设补，**填了**的字段逐字段覆盖它——" +
                    "所以「改默认」就是直接改下面的字段，改完留空又能回到默认。",
            )
            actionRow {
                secondaryButton("载入预设内容到表单") {
                    // 先同步预设清单（服务端新增预设时下拉自动补上），再让服务端合并出完整档案
                    sm.syncSoulPresets(ctx, form.serverBase)
                    sm.loadDefaultSoul(ctx, form.serverBase) { form.markDirty() }
                }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("清空人格字段") { sm.clearSoulProfile { form.markDirty() } }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("预览最终提示词") { sm.previewSoul(ctx, form.serverBase) }
            }
            vif({ sm.soulPresetMsg.isNotEmpty() }) {
                textNote(sm.soulPresetMsg, micro = true)
            }
            // 只读预览：官方 TextArea 无 read-only 属性，这里用 no-op 回调（输入不改状态，
            // 下次重组即恢复），标题明确写"预览"。要改人格请改下面的字段。
            vif({ sm.soulPreviewText.isNotEmpty() }) {
                labeledTextArea(
                    "最终提示词（预览，只读）",
                    { sm.soulPreviewText },
                    { },
                    height = 220f,
                    help = "实际发给大模型的完整 instructions，以及哪些字段来自预设、哪些被你覆盖。" +
                        "同时充当保存前校验器：预设拼错会在这里直接报错。",
                )
            }
            dividerH()
            textNote("可在任意字段里引用 {{name}} / {{address_user}} / {{device_id}}；写错变量名会在保存时直接报错（不会把 {{xxx}} 塞给模型）。", micro = true)
            labeledField(
                "自称", { sm.soulName }, { sm.soulName = it; form.markDirty() }, "小智",
                help = "它怎么称呼自己，也决定 {{name}} 的取值。",
            )
            labeledTextArea(
                "自我定位",
                { sm.soulSelfIntro },
                { sm.soulSelfIntro = it; form.markDirty() },
                height = 70f,
                help = "一句话说清「我是谁、我为什么在这里」，是最能定调的一段。",
            )
            labeledField(
                "形象 / 物种", { sm.soulForm }, { sm.soulForm = it; form.markDirty() }, "如：住在音箱里的小狐狸",
                help = "外形与物种设定，影响它描述自己与举例的方式。",
            )
            labeledField(
                "年龄感", { sm.soulAgeFeel }, { sm.soulAgeFeel = it; form.markDirty() }, "如：十来岁",
                help = "用「十来岁」「三十上下」这类感觉型描述，不要写具体数字。",
            )
            labeledField(
                "语气", { sm.soulTone }, { sm.soulTone = it; form.markDirty() }, "如：轻快、温和",
                help = "说话的整体口吻（轻快 / 沉稳 / 俏皮），会作为表达风格写进提示词。",
            )
            labeledField(
                "称呼用户", { sm.soulAddressUser }, { sm.soulAddressUser = it; form.markDirty() }, "你",
                help = "怎么称呼用户（你 / 您 / 主人…），也决定 {{address_user}} 的取值。",
            )
            labeledTextArea(
                "性格",
                { sm.soulTraits },
                { sm.soulTraits = it; form.markDirty() },
                height = 80f,
                help = "每行一条，格式「标签：遇到 X 会怎么做」。写成行为规则比形容词有效得多：" +
                    "例如「好奇：遇到没听过的词会追问一句」。",
            )
            labeledTextArea(
                "价值观", { sm.soulValues }, { sm.soulValues = it; form.markDirty() },
                height = 70f,
                help = "每行一条。它认为重要的事（诚实、陪伴、不打扰…），影响取舍与拒绝方式。",
            )
            labeledTextArea(
                "红线 / 拒绝清单",
                { sm.soulBoundaries },
                { sm.soulBoundaries = it; form.markDirty() },
                height = 70f,
                help = "每行一条。明确不许做的事——这是软约束（写进提示词），不是安全机制：真正的护栏在服务端。",
            )
            labeledTextArea(
                "口头禅", { sm.soulCatchphrases }, { sm.soulCatchphrases = it; form.markDirty() },
                height = 60f,
                help = "每行一条。适度即可：语音回复很短，口头禅太多会显得机械。",
            )
            labeledTextArea(
                "世界观", { sm.soulWorldview }, { sm.soulWorldview = it; form.markDirty() },
                height = 70f,
                help = "它理解世界的方式（住在设备里、能听见整个房间…），让回答保持一致的想象边界。",
            )
            labeledTextArea(
                "来历", { sm.soulBackstory }, { sm.soulBackstory = it; form.markDirty() },
                height = 70f,
                help = "简短背景故事，用户问「你是谁呀」时用得上。",
            )
            labeledTextArea(
                "与用户的关系起点",
                { sm.soulRelationshipOrigin },
                { sm.soulRelationshipOrigin = it; form.markDirty() },
                height = 70f,
                help = "最初怎么认识的、关系基调如何，影响亲密度与分寸。",
            )
            labeledTextArea(
                "风格范例",
                { sm.soulExamples },
                { sm.soulExamples = it; form.markDirty() },
                height = 90f,
                help = "每行一组问答示例。锁定说话风格最有效的一招——比形容词管用得多。",
            )
            dividerH()
            // 语音第一性原则：短。这两个数字会作为**硬约束**写进提示词。
            labeledIntField(
                "回复最多句数", { sm.soulMaxSentences }, { sm.soulMaxSentences = it; form.markDirty() }, "2",
                help = "1~8，作为硬约束写进提示词。语音场景建议 1~2 句：说不完的内容用户也记不住。",
            )
            labeledIntField(
                "回复最多字数", { sm.soulMaxChars }, { sm.soulMaxChars = it; form.markDirty() }, "40",
                help = "10~500，作为硬约束写进提示词。超出会被截断，建议 40 字上下。",
            )
            switchRow(
                "说人话（短句、可省略主语）", sm.soulColloquial == "true",
                help = "强制口语化表达：不用书面语、不用列表、允许省略主语，更适合被「听」而不是被「读」。",
            ) {
                form.toggleStringBool(sm::soulColloquial)
            }
            switchRow(
                "允许 emoji", sm.soulEmoji == "true",
                help = "⚠️ 默认关闭：语音合成会把 emoji 直接念出来（「微笑表情」），除非确知当前 TTS 会跳过。",
            ) {
                form.toggleStringBool(sm::soulEmoji)
            }
            dividerH()
            labeledTextArea(
                "当前情境", { sm.soulScenario }, { sm.soulScenario = it; form.markDirty() },
                height = 60f,
                help = "此刻的场景说明（如「晚上，用户在卧室」），用于调整话题与音量感。",
            )
            labeledTextArea(
                "设备提示", { sm.soulDeviceHint }, { sm.soulDeviceHint = it; form.markDirty() },
                height = 60f,
                help = "告诉模型它跑在什么设备上（有无屏幕、能否播放音乐），避免给出做不到的回答。",
            )
        }
    }
}

/**
 * 记忆（`[memory]` 图记忆）配置卡。
 *
 * 信息架构（借 DSH 的分工，见 plugin 文档 §5）：
 * **写页**（开关/参数/抽取模型）= 配置动作；**读页**（上面的状态行 + 测试召回）= 验证效果。
 * 没有"测试召回"这个反馈，记忆这种看不见的功能一定会被判定为"开了没用"。
 */
fun ViewContainer<*, *>.memoryConfigCard(form: ConfigFormState, ctx: Pager) {
    val sm = form.sm
    groupedCard("记忆（本地图记忆）") {
        switchRow(
            "启用记忆", sm.memoryEnabled == "true",
            help = "默认关闭 = 零依赖零成本（行为与没有记忆时完全一致）。打开后：对话原文写入本地 SQLite，" +
                "后台用一次辅助模型调用抽取摘要，下一轮提问时本地词法召回、注入**原始问答**作为背景资料。",
        ) {
            form.toggleStringBool(sm::memoryEnabled)
        }
        vif({ sm.memoryEnabled == "true" }) {
            dividerH()
            textNote(
                "⚠️ 记忆只在本地流水线生效：启用 AIUI 全链路时对话走云端闭环，人格与记忆都不参与。",
                tone = "warn",
                micro = true,
            )
            // ===== 状态（读页）：先让用户看到"到底有没有在工作" =====
            actionRow {
                secondaryButton("刷新状态") { sm.refreshStatus(ctx, form.serverBase) }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("立即维护") { sm.maintain(ctx, form.serverBase, dryRun = false) }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("演练保留策略") { sm.maintain(ctx, form.serverBase, dryRun = true) }
            }
            vif({ sm.memoryStatusMsg.isNotEmpty() }) {
                textNote(sm.memoryStatusMsg, micro = true)
            }
            vif({ sm.memoryMaintainMsg.isNotEmpty() }) {
                textNote(sm.memoryMaintainMsg, micro = true)
            }
            dividerH()
            // ===== 开关 =====
            switchRow(
                "启用召回", sm.memoryRecallEnabled == "true",
                help = "每轮提问时从记忆库检索相关片段并注入上下文。关闭 = 只存不用（适合先积累）。",
            ) {
                form.toggleStringBool(sm::memoryRecallEnabled)
            }
            switchRow(
                "启用后台抽取", sm.memoryExtractionEnabled == "true",
                help = "对话结束后用辅助模型把原文压成摘要与事实条目（会产生额外调用与费用）。" +
                    "关闭则只存对话原文、不生成记忆条目。",
            ) {
                form.toggleStringBool(sm::memoryExtractionEnabled)
            }
            dividerH()
            // ===== 召回参数 =====
            labeledField(
                "库文件路径", { sm.memoryDbPath }, { sm.memoryDbPath = it; form.markDirty() }, "/data/data.db",
                help = "SQLite 库文件路径。必须落在挂载卷内（默认 /data），否则容器重建即丢全部记忆。",
            )
            labeledField(
                "作用域前缀", { sm.memoryScopePrefix }, { sm.memoryScopePrefix = it; form.markDirty() }, "xiaozhi",
                help = "记忆按「前缀 + 设备号」隔离。同一前缀下的设备共享一份记忆；换前缀等于换一套库。",
            )
            labeledIntField(
                "近况轮数", { sm.memoryFreshTurnCount }, { sm.memoryFreshTurnCount = it; form.markDirty() }, "5",
                help = "始终带入最近 N 轮原文（不经过检索），保证短期上下文连贯。调大更连贯，但更费 token。",
            )
            // ===== 高级（检索预算与库运维调优，默认折叠；P0-1）=====
            advancedToggleRow(form)
            vif({ form.showAdvanced }) {
                labeledIntField(
                    "最多召回条数", { sm.memoryRecallMaxNodes }, { sm.memoryRecallMaxNodes = it; form.markDirty() }, "6",
                    help = "一次召回最多注入多少条记忆；条数越多越全面，但会挤占上下文与 token 预算。",
                )
                labeledIntField(
                    "注入 token 预算", { sm.memoryRecallMaxTokens }, { sm.memoryRecallMaxTokens = it; form.markDirty() }, "400",
                    help = "召回内容注入上下文的 token 上限，超出即截断（按相关度从高到低保留）。",
                )
                labeledIntField(
                    "召回耗时预算", { sm.memoryRecallBudgetMs }, { sm.memoryRecallBudgetMs = it; form.markDirty() }, "300",
                    help = "检索超过该耗时（ms）即放弃本次召回、退化为只带近况——保证记忆不会拖慢回答。",
                )
                labeledIntField(
                    "维护间隔（轮）", { sm.memoryMaintenanceInterval }, { sm.memoryMaintenanceInterval = it; form.markDirty() }, "6",
                    help = "每 N 轮做一次后台维护（合并、淘汰、按保留策略清理）。0 = 关闭自动维护。",
                )
                labeledIntField(
                    "抽取最大尝试次数", { sm.memoryQuarantineMaxAttempts }, { sm.memoryQuarantineMaxAttempts = it; form.markDirty() }, "3",
                    help = "抽取失败后最多重试几次，超过则把该条隔离、不再重试（避免反复烧钱）。",
                )
            }
            dropdownField(
                label = "记忆引擎",
                currentLabel = {
                    sm.memoryBackendOptions.firstOrNull { it.first == sm.memoryBackend }?.second ?: sm.memoryBackend
                },
                options = { sm.memoryBackendOptions },
                selectedId = { sm.memoryBackend },
                isOpen = { form.isDropdownOpen("memory_backend") },
                onToggle = { form.toggleDropdown("memory_backend") },
                onSelect = {
                    sm.memoryBackend = it
                    form.markDirty()
                    form.openDropdown = ""
                },
                help = "内置图记忆 = 抽取摘要 + 图结构召回（推荐）；" +
                    "纯存储 = 只把原文落库，零辅助模型成本，适合先积累、之后切回来再决定是否重建；" +
                    "本地文档 RAG = 扫描目录里的 txt/md 建词法索引（零 LLM、零外网），对话原文仍照常落库；" +
                    "不生效 = 保留配置但不参与运行时。",
            )
            vif({ sm.memoryBackend == "rag" }) {
                // ===== rag 参数组（P1-9，仅 rag 选中时展示）=====
                fieldLabel(
                    "文档索引",
                    help = "把下面目录里的 txt/md 切块建词法索引（零 LLM、零外网），提问时检索分块注入上下文。" +
                        "目录为空 = 零知识、对话完全正常。改目录或文件后需重建索引。",
                    trailing = { secondaryButton("重建索引") { sm.reindexRag(ctx, form.serverBase) } },
                )
                vif({ sm.memoryRagReindexMsg.isNotEmpty() }) {
                    textNote(sm.memoryRagReindexMsg, micro = true)
                }
                labeledField(
                    "文档目录", { sm.memoryRagDocsDir }, { sm.memoryRagDocsDir = it; form.markDirty() }, "/data/rag-docs",
                    help = "放 txt / md 文档的目录（需在挂载卷内）。子目录会一并扫描。",
                )
                labeledIntField(
                    "分块长度", { sm.memoryRagChunkSize }, { sm.memoryRagChunkSize = it; form.markDirty() }, "512",
                    help = "每块字符数：太大召回的段落冗长、太小容易断章取义。",
                )
                labeledIntField(
                    "分块重叠", { sm.memoryRagChunkOverlap }, { sm.memoryRagChunkOverlap = it; form.markDirty() }, "64",
                    help = "相邻块重叠的字符数，避免关键句正好被切在边界上。",
                )
                labeledIntField(
                    "每次召回条数", { sm.memoryRagTopK }, { sm.memoryRagTopK = it; form.markDirty() }, "4",
                    help = "一次检索返回多少个分块注入上下文；越多越全面，也越费 token。",
                )
            }
            dropdownField(
                label = "记忆注入位置",
                currentLabel = {
                    sm.memoryInjectPositionOptions.firstOrNull { it.first == sm.memoryInjectPosition }?.second
                        ?: sm.memoryInjectPosition
                },
                options = { sm.memoryInjectPositionOptions },
                selectedId = { sm.memoryInjectPosition },
                isOpen = { form.isDropdownOpen("memory_inject") },
                onToggle = { form.toggleDropdown("memory_inject") },
                onSelect = {
                    sm.memoryInjectPosition = it
                    form.markDirty()
                    form.openDropdown = ""
                },
                help = "「历史之后」能保住「人格 + 历史」这段前缀的缓存命中（推荐）；" +
                    "只有当上下文历史轮数调得很大、导致记忆离当前问题太远时，才考虑换成「历史之前」。",
            )
            dividerH()
            // ===== 抽取模型（独立路由；留空 = 复用 [llm]）=====
            fieldLabel(
                "抽取模型",
                help = "后台抽取摘要所用的大模型，留空 = 复用 LLM 卡的配置。建议指向更便宜的小模型：" +
                    "抽取只求结构化，不需要多聪明。",
            )
            labeledField(
                "接口地址", { sm.memoryExtractorApiBase }, { sm.memoryExtractorApiBase = it; form.markDirty() },
                help = "OpenAI 兼容端点；留空则复用 LLM 卡的接口地址。",
            )
            labeledField(
                "API Key",
                { sm.memoryExtractorApiKey },
                { sm.memoryExtractorApiKey = it; form.markDirty() },
                if (sm.memoryExtractorApiKeyConfigured) "已配置（留空则不修改）" else "未配置",
                help = "留空 = 复用 LLM 卡的密钥；已配置时留空保存不会覆盖。",
            )
            labeledField(
                "模型名", { sm.memoryExtractorModel }, { sm.memoryExtractorModel = it; form.markDirty() },
                help = "抽取用的模型标识；留空则复用 LLM 卡。",
            )
            labeledDecimalField(
                "随机性（temperature）", { sm.memoryExtractorTemperature }, { sm.memoryExtractorTemperature = it; form.markDirty() }, "0.1",
                help = "抽取要求稳定结构化，建议保持很低（0.1 左右）。",
            )
            dividerH()
            // ===== 保留策略 =====
            dropdownField(
                label = "原文保留策略",
                currentLabel = {
                    sm.retentionKeepOptions.firstOrNull { it.first == sm.memoryRetentionKeep }?.second
                        ?: sm.memoryRetentionKeep
                },
                options = { sm.retentionKeepOptions },
                selectedId = { sm.memoryRetentionKeep },
                isOpen = { form.isDropdownOpen("memory_keep") },
                onToggle = { form.toggleDropdown("memory_keep") },
                onSelect = {
                    sm.memoryRetentionKeep = it
                    form.markDirty()
                    form.openDropdown = ""
                },
                help = "被记忆条目引用的原文**永不删除**（只清理未被引用的原文）。" +
                    "先点「演练保留策略」看清会处理多少条再执行。",
            )
            labeledIntField(
                "保留最近轮数", { sm.memoryRetentionRecentTurns }, { sm.memoryRetentionRecentTurns = it; form.markDirty() }, "0",
                help = "最近 N 轮原文无论如何都保留；0 = 不额外保留。",
            )
            labeledIntField(
                "保留天数", { sm.memoryRetentionDays }, { sm.memoryRetentionDays = it; form.markDirty() }, "0",
                help = "超过该天数的未引用原文会被清理；0 = 不按时间清理。",
            )
            switchRow(
                "只演练不删除", sm.memoryRetentionDryRun == "true",
                help = "自动维护时只统计、不真正删除（全局演练开关）。手动「演练保留策略」始终不删除。",
            ) {
                form.toggleStringBool(sm::memoryRetentionDryRun)
            }
            dividerH()
            // ===== 测试召回（最重要的可用性反馈）=====
            labeledField(
                "试召回的一句话", { sm.memoryTestText }, { sm.memoryTestText = it },
                help = "填一句你真实说过的话，看它能召回什么——这是唯一能验证「记忆到底有没有用」的入口。",
                trailing = { secondaryButton("测试召回") { sm.testRecall(ctx, form.serverBase) } },
            )
            labeledField(
                "召回作用域", { sm.memoryTestScope }, { sm.memoryTestScope = it },
                help = "留空 = 查全部设备；填「前缀:设备号」可只看某台设备的记忆。",
            )
            vif({ sm.memoryTestMsg.isNotEmpty() }) {
                textNote(
                    sm.memoryTestMsg,
                    tone = if (sm.memoryTestOk) "tertiary" else "error",
                    micro = true,
                )
            }
            dividerH()
            // ===== 清空（两段式强确认，不可恢复）=====
            textNote("清空记忆（不可恢复）：先填作用域，或勾选「清空全部设备」，再点两次按钮确认。", micro = true)
            labeledField(
                "清空的作用域", { sm.memoryClearScope }, { sm.memoryClearScope = it; sm.memoryClearConfirm = false },
                help = "填「xiaozhi:设备号」只清该设备；留空且未勾选全部设备时不会执行。",
            )
            switchRow("清空全部设备", sm.memoryClearAll) {
                sm.memoryClearAll = !sm.memoryClearAll
                sm.memoryClearConfirm = false
            }
            actionRow {
                secondaryButton("清空记忆") { sm.clear(ctx, form.serverBase) }
                vif({ sm.memoryClearMsg.isNotEmpty() }) {
                    View { attr { width(AdminSpace.sm) } }
                    textNote(sm.memoryClearMsg, tone = "error", micro = true)
                }
            }
        }
        vif({ sm.memoryEnabled != "true" }) {
            textNote("未启用：不建库、不读盘、不调用任何模型（零成本）。", micro = true)
        }
    }
}
