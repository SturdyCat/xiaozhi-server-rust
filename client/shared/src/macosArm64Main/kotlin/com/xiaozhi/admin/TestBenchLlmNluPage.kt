package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.labeledTextArea
import com.xiaozhi.admin.kit.statusBadge
import com.xiaozhi.admin.kit.twoPane

/**
 * 测试台 LLM 对话 / NLU 意图两个 tab（macOS；旧 `TestBenchNluPage.kt` 并入后删除）。
 *
 * 流程口径（与旧实现一致）：
 * - LLM：发 `llm_test` → 服务端按最新保存的 `[llm]` 配置直调 LLM → 回包携带回复与
 *   服务端耗时（改配置后无需重启即可验证）；
 * - NLU：发 `nlu_test` → 服务端按磁盘上最新 `[nlu]` 配置构建 TinyBERT 直推一句 →
 *   回调 {state, intent, confidence, tokenizeMs, inferMs, text}。
 */
@Composable
internal fun BenchLlmPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            LabeledCard(title = "LLM 对话测试") {
                HintLine("流程：发送提示词 → 服务器按最新保存的 [llm] 配置直调 LLM → 返回回复（改配置后无需重启即可验证）")
                labeledTextArea(
                    label = "测试提示词",
                    value = bench.llmPrompt,
                    onValueChange = { bench.llmPrompt = it },
                    minLines = 3,
                )
                actionRow {
                    Button(
                        text = if (bench.llmBusy) "测试中…" else "发送测试",
                        onClick = { bench.testLlm(pager) },
                        theme = ButtonTheme.PRIMARY,
                        type = ButtonType.FILL,
                        loading = bench.llmBusy,
                        disabled = bench.llmBusy,
                    )
                }
                // 状态徽标：独立一行
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.sm),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    when {
                        bench.llmBusy -> statusBadge("busy", "测试中")
                        bench.llmReply.isNotEmpty() && bench.llmOk ->
                            statusBadge("connected", "正常 · ${bench.llmElapsedMs}ms")
                        bench.llmReply.isNotEmpty() -> statusBadge("error", "失败")
                        else -> statusBadge("idle", "空闲")
                    }
                }
                dividerH()
                BenchResultBlock(
                    placeholder = "回复将显示在此（右侧配置卡可修改 backend/api_base/api_key/model）",
                    body = bench.llmReply,
                    bodyError = !bench.llmOk,
                    detail = if (bench.llmOk && bench.llmElapsedMs > 0) "服务端耗时 ${bench.llmElapsedMs} ms" else "",
                    copyHint = bench.llmCopyHint,
                    onCopy = { bench.copyLlm(pager) },
                )
            }
        },
        right = {
            Column(modifier = Modifier.fillMaxWidth()) {
                LlmConfigCard(form)
            }
        },
    )
}

@Composable
internal fun BenchNluPage(
    bench: TestBenchState,
    form: ConfigFormState,
    pager: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            LabeledCard(title = "NLU 意图测试") {
                HintLine(
                    "流程：发送文本 → 服务器按最新保存的 [nlu] 配置构建 TinyBERT 直推 → 返回意图与耗时" +
                        "（改配置后无需重启即可验证；未配置时回错误原因）",
                )
                labeledField(
                    label = "测试文本",
                    value = bench.nluInput,
                    onValueChange = { bench.nluInput = it },
                    placeholder = "把客厅的灯打开",
                )
                actionRow {
                    Button(
                        text = if (bench.nluBusy) "测试中…" else "发送测试",
                        onClick = { bench.testNlu(pager) },
                        theme = ButtonTheme.PRIMARY,
                        type = ButtonType.FILL,
                        loading = bench.nluBusy,
                        disabled = bench.nluBusy,
                    )
                }
                // 状态徽标：独立一行
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.sm),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    when {
                        bench.nluBusy -> statusBadge("busy", "测试中")
                        bench.nluOk && bench.nluIntent.isNotEmpty() ->
                            statusBadge("connected", "命中 · ${bench.nluIntent}")
                        bench.nluText.isNotEmpty() ->
                            statusBadge(if (bench.nluOk) "idle" else "error", if (bench.nluOk) "未命中" else "失败")
                        else -> statusBadge("idle", "空闲")
                    }
                }
                dividerH()
                BenchResultBlock(
                    placeholder = "结果将显示在此（右侧配置卡可修改 bert 模型/词表/标签与阈值）",
                    body = bench.nluResultText(),
                    bodyError = !bench.nluOk,
                    // 推理耗时明细：nlu_test 回包携带（本地 TinyBERT 分词/推理分开计时）
                    detail = if (bench.nluOk && (bench.nluTokenizeMs > 0 || bench.nluInferMs > 0)) {
                        "分词 ${bench.nluTokenizeMs} ms / 推理 ${bench.nluInferMs} ms"
                    } else "",
                    copyHint = bench.nluCopyHint,
                    onCopy = { bench.copyNlu(pager) },
                )
            }
        },
        right = {
            Column(modifier = Modifier.fillMaxWidth()) {
                NluConfigCard(form)
            }
        },
    )
}
