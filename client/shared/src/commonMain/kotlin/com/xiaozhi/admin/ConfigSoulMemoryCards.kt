package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

// ============================================================
// 「灵魂」「记忆」两个配置卡（从 ConfigCards.kt 拆出，见 AGENTS.md §5.10）
// ============================================================
// 设计依据：docs/soul-and-graph-memory-plan.md §7（tab 布局 + 交互语义）。
// 表单字段状态在 `ConfigContextState`（`form.sm`）；JSON 往返见其 `putJson`/`fill`。
// ⚠️ 新增 UI 一律复用 AdminTheme/AdminFormControls 里的官方组件组合，禁止自造样式。
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
    groupedCard("灵魂（人格档案）") {
        switchRow("enabled（启用人格档案）", sm.soulEnabled == "true") {
            sm.soulEnabled = if (sm.soulEnabled == "true") "false" else "true"
            form.dirty = true
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text(
                    "关闭时系统提示词 = 上面的 [llm].system_prompt（原文，可随时回滚）。" +
                        "打开后由「平台约束 + 人格 + 表达风格 + [llm].system_prompt」按序组成；保存后**新会话生效**。",
                )
            }
        }
        // ===== 未启用：一键用内置默认人格开启（开箱可用）=====
        vif({ sm.soulEnabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("内置了一份默认人格「小智」：打开就能用，之后每个字段都可以改，也能整份换成自己的。")
                }
            }
            actionRow {
                secondaryButton("用内置默认人格启用") {
                    sm.soulPreset = "xiaozhi"
                    sm.soulEnabled = "true"
                    form.dirty = true
                    sm.soulPresetMsg =
                        "已启用内置默认人格：你没填的字段由预设补，填了的字段覆盖它。" +
                            "点「预览最终提示词」看实际发出去的内容。"
                }
            }
            vif({ sm.soulPresetMsg.isNotEmpty() }) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textSecondary)
                        text(sm.soulPresetMsg)
                    }
                }
            }
        }
        vif({ sm.soulEnabled == "true" }) {
            dividerH()
            // ===== 内置预设（默认灵魂）=====
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("内置默认灵魂：**留空**的字段由预设补，**填了**的字段逐字段覆盖它——" +
                        "所以「改默认」就是直接改下面的字段，改完留空又能回到默认。")
                }
            }
            dropdownField(
                label = "preset（内置人格预设）",
                currentLabel = {
                    sm.soulPresetOptions.firstOrNull { it.first == sm.soulPreset }?.second ?: sm.soulPreset
                },
                options = { sm.soulPresetOptions },
                selectedId = { sm.soulPreset },
                isOpen = { form.openDropdown == "soul_preset" },
                onToggle = {
                    form.openDropdown = if (form.openDropdown == "soul_preset") "" else "soul_preset"
                },
                onSelect = {
                    sm.soulPreset = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            actionRow {
                secondaryButton("载入预设内容到表单") {
                    // 先同步预设清单（服务端新增预设时下拉自动补上），再让服务端合并出完整档案
                    sm.syncSoulPresets(ctx, form.serverBase)
                    sm.loadDefaultSoul(ctx, form.serverBase) { form.dirty = true }
                }
                secondaryButton("清空人格字段") { sm.clearSoulProfile { form.dirty = true } }
            }
            actionRow {
                secondaryButton("预览最终提示词") { sm.previewSoul(ctx, form.serverBase) }
            }
            vif({ sm.soulPresetMsg.isNotEmpty() }) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textSecondary)
                        text(sm.soulPresetMsg)
                    }
                }
            }
            // 只读预览：官方 TextArea 无 read-only 属性，这里用 no-op 回调（输入不改状态，
            // 下次重组即恢复），标题明确写"预览"。要改人格请改下面的字段。
            vif({ sm.soulPreviewText.isNotEmpty() }) {
                labeledTextArea(
                    "最终提示词（预览，只读）",
                    { sm.soulPreviewText },
                    { },
                    height = 220f,
                )
            }
            dividerH()
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("可在任意字段里引用 {{name}} / {{address_user}} / {{device_id}}；写错变量名会在保存时直接报错（不会把 {{xxx}} 塞给模型）。")
                }
            }
            labeledField("name（自称）", { sm.soulName }, { sm.soulName = it; form.dirty = true }, "小智")
            labeledTextArea(
                "self_intro（自我定位）",
                { sm.soulSelfIntro },
                { sm.soulSelfIntro = it; form.dirty = true },
                height = 70f,
            )
            labeledField("form（形象/物种）", { sm.soulForm }, { sm.soulForm = it; form.dirty = true }, "如：住在音箱里的小狐狸")
            labeledField("age_feel（年龄感）", { sm.soulAgeFeel }, { sm.soulAgeFeel = it; form.dirty = true }, "如：十来岁")
            labeledField("tone（语气）", { sm.soulTone }, { sm.soulTone = it; form.dirty = true }, "如：轻快、温和")
            labeledField("address_user（称呼用户）", { sm.soulAddressUser }, { sm.soulAddressUser = it; form.dirty = true }, "你")
            labeledTextArea(
                "traits（性格：每行「标签：遇到 X 会怎么做」）",
                { sm.soulTraits },
                { sm.soulTraits = it; form.dirty = true },
                height = 80f,
            )
            labeledTextArea("values（价值：每行一条）", { sm.soulValues }, { sm.soulValues = it; form.dirty = true }, height = 70f)
            labeledTextArea(
                "boundaries（红线/拒绝清单：每行一条）",
                { sm.soulBoundaries },
                { sm.soulBoundaries = it; form.dirty = true },
                height = 70f,
            )
            labeledTextArea("catchphrases（口头禅：每行一条）", { sm.soulCatchphrases }, { sm.soulCatchphrases = it; form.dirty = true }, height = 60f)
            labeledTextArea("worldview（世界观）", { sm.soulWorldview }, { sm.soulWorldview = it; form.dirty = true }, height = 70f)
            labeledTextArea("backstory（来历）", { sm.soulBackstory }, { sm.soulBackstory = it; form.dirty = true }, height = 70f)
            labeledTextArea(
                "relationship_origin（与用户的关系起点）",
                { sm.soulRelationshipOrigin },
                { sm.soulRelationshipOrigin = it; form.dirty = true },
                height = 70f,
            )
            labeledTextArea(
                "examples（风格范例：每行一组 Q/A，锁风格最有效）",
                { sm.soulExamples },
                { sm.soulExamples = it; form.dirty = true },
                height = 90f,
            )
            dividerH()
            // 语音第一性原则：短。这两个数字会作为**硬约束**写进提示词。
            labeledField("max_sentences（最多句数 1~8）", { sm.soulMaxSentences }, { sm.soulMaxSentences = it; form.dirty = true }, "2")
            labeledField("max_chars（最多字数 10~500）", { sm.soulMaxChars }, { sm.soulMaxChars = it; form.dirty = true }, "40")
            switchRow("colloquial（说人话：短句、可省略主语）", sm.soulColloquial == "true") {
                sm.soulColloquial = if (sm.soulColloquial == "true") "false" else "true"
                form.dirty = true
            }
            switchRow("emoji（允许 emoji）", sm.soulEmoji == "true") {
                sm.soulEmoji = if (sm.soulEmoji == "true") "false" else "true"
                form.dirty = true
            }
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.warningTintText)
                    text("⚠️ emoji 默认关闭：语音合成会把 emoji 直接念出来。")
                }
            }
            dividerH()
            labeledTextArea("scenario（当前情境）", { sm.soulScenario }, { sm.soulScenario = it; form.dirty = true }, height = 60f)
            labeledTextArea("device_hint（设备提示）", { sm.soulDeviceHint }, { sm.soulDeviceHint = it; form.dirty = true }, height = 60f)
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
        switchRow("enabled（启用记忆）", sm.memoryEnabled == "true") {
            sm.memoryEnabled = if (sm.memoryEnabled == "true") "false" else "true"
            form.dirty = true
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text(
                    "默认关闭 = 零依赖零成本（行为与没有记忆时完全一致）。打开后：对话原文写入本地 SQLite，" +
                        "后台用一次辅助模型调用抽取摘要，下一轮提问时本地词法召回、注入**原始问答**作为背景资料。",
                )
            }
        }
        vif({ sm.memoryEnabled == "true" }) {
            dividerH()
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.warningTintText)
                    text("⚠️ 记忆只在本地流水线生效：启用 AIUI 全链路时对话走云端闭环，人格与记忆都不参与。")
                }
            }
            // ===== 状态（读页）：先让用户看到"到底有没有在工作" =====
            actionRow {
                secondaryButton("刷新状态") { sm.refreshStatus(ctx, form.serverBase) }
                secondaryButton("立即维护") { sm.maintain(ctx, form.serverBase, dryRun = false) }
                secondaryButton("演练保留策略") { sm.maintain(ctx, form.serverBase, dryRun = true) }
            }
            vif({ sm.memoryStatusMsg.isNotEmpty() }) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textSecondary)
                        text(sm.memoryStatusMsg)
                    }
                }
            }
            vif({ sm.memoryMaintainMsg.isNotEmpty() }) {
                Text {
                    attr {
                        fontSize(AdminType.micro)
                        color(AdminColors.textTertiary)
                        text(sm.memoryMaintainMsg)
                    }
                }
            }
            dividerH()
            // ===== 开关 =====
            switchRow("recall_enabled（启用召回）", sm.memoryRecallEnabled == "true") {
                sm.memoryRecallEnabled = if (sm.memoryRecallEnabled == "true") "false" else "true"
                form.dirty = true
            }
            switchRow("extraction_enabled（启用抽取）", sm.memoryExtractionEnabled == "true") {
                sm.memoryExtractionEnabled = if (sm.memoryExtractionEnabled == "true") "false" else "true"
                form.dirty = true
            }
            dividerH()
            // ===== 召回参数 =====
            labeledField("db_path（库文件，须在挂载卷内）", { sm.memoryDbPath }, { sm.memoryDbPath = it; form.dirty = true }, "/data/memory.db")
            labeledField("scope_prefix（作用域前缀）", { sm.memoryScopePrefix }, { sm.memoryScopePrefix = it; form.dirty = true }, "xiaozhi")
            labeledField("fresh_turn_count（近况轮数）", { sm.memoryFreshTurnCount }, { sm.memoryFreshTurnCount = it; form.dirty = true }, "5")
            labeledField("recall_max_nodes（最多召回条数）", { sm.memoryRecallMaxNodes }, { sm.memoryRecallMaxNodes = it; form.dirty = true }, "6")
            labeledField("recall_max_tokens（注入 token 预算）", { sm.memoryRecallMaxTokens }, { sm.memoryRecallMaxTokens = it; form.dirty = true }, "400")
            labeledField("recall_budget_ms（召回耗时预算）", { sm.memoryRecallBudgetMs }, { sm.memoryRecallBudgetMs = it; form.dirty = true }, "300")
            labeledField("maintenance_interval（维护间隔/轮）", { sm.memoryMaintenanceInterval }, { sm.memoryMaintenanceInterval = it; form.dirty = true }, "6")
            labeledField("quarantine_max_attempts（抽取最大尝试）", { sm.memoryQuarantineMaxAttempts }, { sm.memoryQuarantineMaxAttempts = it; form.dirty = true }, "3")
            dropdownField(
                label = "engine（引擎）",
                currentLabel = {
                    sm.memoryBackendOptions.firstOrNull { it.first == sm.memoryBackend }?.second ?: sm.memoryBackend
                },
                options = { sm.memoryBackendOptions },
                selectedId = { sm.memoryBackend },
                isOpen = { form.openDropdown == "memory_backend" },
                onToggle = { form.openDropdown = if (form.openDropdown == "memory_backend") "" else "memory_backend" },
                onSelect = {
                    sm.memoryBackend = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            dropdownField(
                label = "inject_position（记忆注入位置）",
                currentLabel = {
                    sm.memoryInjectPositionOptions.firstOrNull { it.first == sm.memoryInjectPosition }?.second
                        ?: sm.memoryInjectPosition
                },
                options = { sm.memoryInjectPositionOptions },
                selectedId = { sm.memoryInjectPosition },
                isOpen = { form.openDropdown == "memory_inject" },
                onToggle = { form.openDropdown = if (form.openDropdown == "memory_inject") "" else "memory_inject" },
                onSelect = {
                    sm.memoryInjectPosition = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    text("「历史之后」保住人格+历史这段前缀的缓存命中；只有在 max_history 调得很大时才考虑换成「历史之前」。")
                }
            }
            dividerH()
            // ===== 抽取模型（独立路由；留空 = 复用 [llm]）=====
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("抽取模型（留空 = 复用 [llm]；建议指向更便宜的小模型）")
                }
            }
            labeledField("extractor.api_base", { sm.memoryExtractorApiBase }, { sm.memoryExtractorApiBase = it; form.dirty = true })
            labeledField(
                "extractor.api_key",
                { sm.memoryExtractorApiKey },
                { sm.memoryExtractorApiKey = it; form.dirty = true },
                if (sm.memoryExtractorApiKeyConfigured) "已配置（留空则不修改）" else "未配置",
            )
            labeledField("extractor.model", { sm.memoryExtractorModel }, { sm.memoryExtractorModel = it; form.dirty = true })
            labeledField("extractor.temperature", { sm.memoryExtractorTemperature }, { sm.memoryExtractorTemperature = it; form.dirty = true }, "0.1")
            dividerH()
            // ===== 保留策略 =====
            dropdownField(
                label = "retention.keep（原文保留策略）",
                currentLabel = {
                    sm.retentionKeepOptions.firstOrNull { it.first == sm.memoryRetentionKeep }?.second
                        ?: sm.memoryRetentionKeep
                },
                options = { sm.retentionKeepOptions },
                selectedId = { sm.memoryRetentionKeep },
                isOpen = { form.openDropdown == "memory_keep" },
                onToggle = { form.openDropdown = if (form.openDropdown == "memory_keep") "" else "memory_keep" },
                onSelect = {
                    sm.memoryRetentionKeep = it
                    form.dirty = true
                    form.openDropdown = ""
                },
            )
            Text {
                attr {
                    fontSize(AdminType.micro)
                    color(AdminColors.textTertiary)
                    text("被轮次记忆引用的原文**永不删除**（仅清理未被引用的原文）；先点「演练保留策略」看清会处理多少条。")
                }
            }
            labeledField("retention.recent_turns", { sm.memoryRetentionRecentTurns }, { sm.memoryRetentionRecentTurns = it; form.dirty = true }, "0")
            labeledField("retention.retention_days", { sm.memoryRetentionDays }, { sm.memoryRetentionDays = it; form.dirty = true }, "0")
            switchRow("retention.dry_run（只演练不删除）", sm.memoryRetentionDryRun == "true") {
                sm.memoryRetentionDryRun = if (sm.memoryRetentionDryRun == "true") "false" else "true"
                form.dirty = true
            }
            dividerH()
            // ===== 测试召回（最重要的可用性反馈）=====
            labeledField("试召回的一句话", { sm.memoryTestText }, { sm.memoryTestText = it })
            labeledField("试召回的作用域（留空 = 全部设备）", { sm.memoryTestScope }, { sm.memoryTestScope = it })
            actionRow {
                secondaryButton("测试召回") { sm.testRecall(ctx, form.serverBase) }
                vif({ sm.memoryTestMsg.isNotEmpty() }) {
                    View { attr { width(AdminSpace.sm) } }
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            color(if (sm.memoryTestOk) AdminColors.textSecondary else AdminColors.dangerTintText)
                            text(sm.memoryTestMsg)
                        }
                    }
                }
            }
            dividerH()
            // ===== 清空（两段式强确认，不可恢复）=====
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text("清空记忆（不可恢复）：先填作用域，或勾选「清空全部设备」，再点两次按钮确认。")
                }
            }
            labeledField("清空的作用域（如 xiaozhi:设备号）", { sm.memoryClearScope }, { sm.memoryClearScope = it; sm.memoryClearConfirm = false })
            switchRow("清空全部设备", sm.memoryClearAll) {
                sm.memoryClearAll = !sm.memoryClearAll
                sm.memoryClearConfirm = false
            }
            actionRow {
                secondaryButton("清空记忆") { sm.clear(ctx, form.serverBase) }
                vif({ sm.memoryClearMsg.isNotEmpty() }) {
                    View { attr { width(AdminSpace.sm) } }
                    Text {
                        attr {
                            fontSize(AdminType.micro)
                            color(AdminColors.dangerTintText)
                            text(sm.memoryClearMsg)
                        }
                    }
                }
            }
        }
        vif({ sm.memoryEnabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：不建库、不读盘、不调用任何模型（零成本）。")
                }
            }
        }
    }
}
