package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.button.ButtonTheme
import com.gearui.foundation.layout.Spacing
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledDecimalField
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.labeledSelect
import com.xiaozhi.admin.kit.switchRow

/**
 * 「记忆」配置卡（`[memory]`）—— Compose 版（从 476 行的 `ConfigSoulMemoryCards.kt` 按职责拆出）。
 *
 * ## 四组不可混淆的动作（与迁移前一致）
 *
 * | 动作 | 端点 | 语义 |
 * | --- | --- | --- |
 * | 刷新状态 | `GET /api/memory/status` | 只读投影：库大小、条数、抽取器、作用域 |
 * | 立即维护 / 演练保留策略 | `POST /api/memory/maintain` | `dry_run` = 只演练不删除 |
 * | 测试召回 | `POST /api/memory/recall` | 当场验证"记忆到底有没有在工作" |
 * | 清空记忆 | `POST /api/memory/clear` | **两段式强确认**：第一次点击只置位并提示 |
 *
 * ## 字段分组
 *
 * ```
 * 启用记忆 → 引擎/库路径 → 召回 → 预算与轮次（高级）
 * 文档 RAG（仅 engine=rag）→ 注入位置 → 抽取模型凭据 → 保留策略
 * 测试召回 → 清空记忆
 * ```
 */
@Composable
fun MemoryConfigCard(form: ConfigFormState) {
    val sm = form.sm
    val ctx = LocalAdminPagerOrThrow()
    LabeledCard(title = "记忆（本地图记忆）") {
        switchRow(
            label = "启用记忆",
            checked = sm.memoryEnabled == "true",
            onToggle = { form.toggleStringBool(sm::memoryEnabled) },
            help = "关闭后不再召回、不再抽取，已有数据保留在库文件里。",
        )

        if (sm.memoryEnabled == "true") {
            actionRow {
                Button(
                    text = "刷新状态",
                    onClick = { sm.refreshStatus(ctx, form.serverBase) },
                    type = ButtonType.OUTLINE,
                )
                Button(
                    text = "立即维护",
                    onClick = { sm.maintain(ctx, form.serverBase, dryRun = false) },
                    type = ButtonType.OUTLINE,
                )
                Button(
                    text = "演练保留策略",
                    onClick = { sm.maintain(ctx, form.serverBase, dryRun = true) },
                    type = ButtonType.OUTLINE,
                )
            }
            if (sm.memoryStatusMsg.isNotEmpty()) HintLine(sm.memoryStatusMsg)
            if (sm.memoryMaintainMsg.isNotEmpty()) HintLine(sm.memoryMaintainMsg)

            labeledSelect(
                label = "记忆引擎",
                selectedId = sm.memoryBackend,
                options = sm.memoryBackendOptions,
                onSelect = {
                    sm.memoryBackend = it
                    form.markDirty()
                },
                help = "内置图记忆 = 本地 SQLite 图谱；纯存储 = 只存原文；本地文档 RAG = 扫描目录建索引；" +
                    "不生效 = 保留配置但不启用。",
            )
            labeledField(
                label = "库文件路径",
                value = sm.memoryDbPath,
                onValueChange = {
                    sm.memoryDbPath = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "作用域前缀",
                value = sm.memoryScopePrefix,
                onValueChange = {
                    sm.memoryScopePrefix = it
                    form.markDirty()
                },
                help = "设备/用户维度的作用域前缀，用于把不同设备的记忆隔开。",
            )
            switchRow(
                label = "启用召回",
                checked = sm.memoryRecallEnabled == "true",
                onToggle = { form.toggleStringBool(sm::memoryRecallEnabled) },
            )
            switchRow(
                label = "启用后台抽取",
                checked = sm.memoryExtractionEnabled == "true",
                onToggle = { form.toggleStringBool(sm::memoryExtractionEnabled) },
                help = "关闭后只记原文、不抽取结构化记忆（先积累再决定时用）。",
            )

            AdvancedToggleRow(form)
            if (form.showAdvanced) {
                labeledIntField(
                    label = "近况轮数",
                    value = sm.memoryFreshTurnCount,
                    onValueChange = {
                        sm.memoryFreshTurnCount = it
                        form.markDirty()
                    },
                    help = "最近 N 轮原文已在本轮上下文里，不再重复召回。",
                )
                labeledIntField(
                    label = "最多召回条数",
                    value = sm.memoryRecallMaxNodes,
                    onValueChange = {
                        sm.memoryRecallMaxNodes = it
                        form.markDirty()
                    },
                )
                labeledIntField(
                    label = "注入 token 预算",
                    value = sm.memoryRecallMaxTokens,
                    onValueChange = {
                        sm.memoryRecallMaxTokens = it
                        form.markDirty()
                    },
                )
                labeledIntField(
                    label = "召回耗时预算",
                    value = sm.memoryRecallBudgetMs,
                    onValueChange = {
                        sm.memoryRecallBudgetMs = it
                        form.markDirty()
                    },
                    help = "单位毫秒；超预算即降级为不注入，避免拖慢首字延迟。",
                )
                labeledIntField(
                    label = "维护间隔（轮）",
                    value = sm.memoryMaintenanceInterval,
                    onValueChange = {
                        sm.memoryMaintenanceInterval = it
                        form.markDirty()
                    },
                )
                labeledIntField(
                    label = "抽取最大尝试次数",
                    value = sm.memoryQuarantineMaxAttempts,
                    onValueChange = {
                        sm.memoryQuarantineMaxAttempts = it
                        form.markDirty()
                    },
                    help = "超过该次数仍失败的抽取进入隔离区，不再无限重试。",
                )
            }

            // ===== 文档 RAG（P1-9，仅 engine=rag 时生效）=====
            if (sm.memoryBackend == "rag") {
                com.xiaozhi.admin.kit.fieldLabelRow(
                    label = "文档目录",
                    help = "扫描该目录下的文档建索引；改动分块参数后需要「重建索引」。",
                    trailing = {
                        Button(
                            text = "重建索引",
                            onClick = { sm.reindexRag(ctx, form.serverBase) },
                            type = ButtonType.OUTLINE,
                        )
                    },
                )
                if (sm.memoryRagReindexMsg.isNotEmpty()) HintLine(sm.memoryRagReindexMsg)
                labeledField(
                    label = "文档目录",
                    value = sm.memoryRagDocsDir,
                    onValueChange = {
                        sm.memoryRagDocsDir = it
                        form.markDirty()
                    },
                )
                if (form.showAdvanced) {
                    labeledIntField(
                        label = "分块长度",
                        value = sm.memoryRagChunkSize,
                        onValueChange = {
                            sm.memoryRagChunkSize = it
                            form.markDirty()
                        },
                    )
                    labeledIntField(
                        label = "分块重叠",
                        value = sm.memoryRagChunkOverlap,
                        onValueChange = {
                            sm.memoryRagChunkOverlap = it
                            form.markDirty()
                        },
                    )
                    labeledIntField(
                        label = "每次召回条数",
                        value = sm.memoryRagTopK,
                        onValueChange = {
                            sm.memoryRagTopK = it
                            form.markDirty()
                        },
                    )
                }
            }

            labeledSelect(
                label = "注入位置",
                selectedId = sm.memoryInjectPosition,
                options = sm.memoryInjectPositionOptions,
                onSelect = {
                    sm.memoryInjectPosition = it
                    form.markDirty()
                },
                help = "历史之后（默认，护住前缀缓存）；历史之前离当前问题更近，但会破掉缓存。",
            )

            com.xiaozhi.admin.kit.fieldLabelRow(
                label = "抽取模型（OpenAI 兼容）",
                help = "用于把对话抽取成结构化记忆；留空则用主 LLM 的配置。",
            )
            labeledField(
                label = "接口地址",
                value = sm.memoryExtractorApiBase,
                onValueChange = {
                    sm.memoryExtractorApiBase = it
                    form.markDirty()
                },
            )
            labeledField(
                label = "API Key",
                value = sm.memoryExtractorApiKey,
                onValueChange = {
                    sm.memoryExtractorApiKey = it
                    form.markDirty()
                },
                placeholder = secretHint(sm.memoryExtractorApiKeyConfigured),
            )
            labeledField(
                label = "模型名",
                value = sm.memoryExtractorModel,
                onValueChange = {
                    sm.memoryExtractorModel = it
                    form.markDirty()
                },
            )
            labeledDecimalField(
                label = "随机性（temperature）",
                value = sm.memoryExtractorTemperature,
                onValueChange = {
                    sm.memoryExtractorTemperature = it
                    form.markDirty()
                },
            )

            // ===== 保留策略 =====
            labeledSelect(
                label = "保留策略",
                selectedId = sm.memoryRetentionKeep,
                options = sm.retentionKeepOptions,
                onSelect = {
                    sm.memoryRetentionKeep = it
                    form.markDirty()
                },
            )
            labeledIntField(
                label = "保留最近轮数",
                value = sm.memoryRetentionRecentTurns,
                onValueChange = {
                    sm.memoryRetentionRecentTurns = it
                    form.markDirty()
                },
            )
            labeledIntField(
                label = "保留天数",
                value = sm.memoryRetentionDays,
                onValueChange = {
                    sm.memoryRetentionDays = it
                    form.markDirty()
                },
                note = if (sm.memoryRetentionDryRun == "true") "只演练不删除已开启：维护时不真正删除" else "",
            )
            switchRow(
                label = "只演练不删除",
                checked = sm.memoryRetentionDryRun == "true",
                onToggle = { form.toggleStringBool(sm::memoryRetentionDryRun) },
            )

            // ===== 测试召回 =====
            labeledField(
                label = "试召回的一句话",
                value = sm.memoryTestText,
                onValueChange = {
                    sm.memoryTestText = it
                    form.markDirty()
                },
                trailing = {
                    Button(
                        text = "测试召回",
                        onClick = { sm.testRecall(ctx, form.serverBase) },
                        type = ButtonType.OUTLINE,
                    )
                },
            )
            labeledField(
                label = "召回作用域",
                value = sm.memoryTestScope,
                onValueChange = { sm.memoryTestScope = it },
                placeholder = "留空 = 跨全部设备",
            )
            if (sm.memoryTestMsg.isNotEmpty()) {
                HintLine(if (sm.memoryTestOk) "✓ ${sm.memoryTestMsg}" else sm.memoryTestMsg)
            }

            // ===== 清空（两段式强确认）=====
            labeledField(
                label = "清空的作用域",
                value = sm.memoryClearScope,
                onValueChange = {
                    sm.memoryClearScope = it
                    // 改作用域即作废上一段确认，避免"确认的是 A、删的是 B"
                    sm.memoryClearConfirm = false
                },
            )
            switchRow(
                label = "清空全部设备",
                checked = sm.memoryClearAll,
                onToggle = {
                    sm.memoryClearAll = it
                    sm.memoryClearConfirm = false
                },
                help = "不可恢复。打开后下面的按钮作用于全部设备的记忆。",
            )
            actionRow {
                Button(
                    text = if (sm.memoryClearConfirm) "确认清空（再点一次）" else "清空记忆",
                    onClick = { sm.clear(ctx, form.serverBase) },
                    type = ButtonType.FILL,
                    theme = ButtonTheme.DANGER,
                )
            }
            if (sm.memoryClearMsg.isNotEmpty()) {
                HintLine(sm.memoryClearMsg)
            }
        } else {
            HintLine("未启用：不再召回、不再抽取，已有数据保留在库文件里。")
        }
    }
}
