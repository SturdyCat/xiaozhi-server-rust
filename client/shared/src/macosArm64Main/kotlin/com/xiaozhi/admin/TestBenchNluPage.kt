package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.SelectableOption
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * NLU 意图测试 tab（连接 / ASR / TTS / LLM 之外的第五个测试端点）：左=测试卡，右=[nlu] 配置卡。
 * 流程：发送 nlu_test → 服务端按磁盘上最新 [nlu] 配置构建 TinyBERT（feature nlu）直推一句
 * → nlu_test 回调 {state, intent, confidence, tokenizeMs, inferMs, text}。
 * 响应式铁律同 [TestBenchPages.kt]：observable 相关文案/分支一律写在 vif/velse 或 attr 闭包内。
 */
fun ViewContainer<*, *>.benchNluPage(
    bench: TestBenchState,
    form: ConfigFormState,
    ctx: Pager,
    wide: () -> Boolean,
) {
    twoPane(
        wide = wide,
        left = {
            groupedCard("NLU 意图测试") {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.textSecondary)
                        marginTop(AdminSpace.xs)
                        text("流程：发送文本 → 服务器按最新保存的 [nlu] 配置构建 TinyBERT 直推 → 返回意图与耗时（改配置后无需重启即可验证；未配置时回错误原因）")
                    }
                }
                labeledField("测试文本", { bench.nluInput }, { bench.nluInput = it }, "把客厅的灯打开")
                actionRow {
                    primaryButton(
                        "发送测试",
                        loading = { bench.nluBusy },
                        loadingText = "测试中…",
                    ) { bench.testNlu(ctx) }
                }
                // 状态徽标：独立一行（每态一个 vif 分支——裸读 when 不随状态更新）
                View {
                    attr {
                        flexDirectionRow()
                        marginTop(AdminSpace.sm)
                    }
                    vif({ bench.nluBusy }) {
                        statusBadge({ "busy" }, { "测试中" })
                    }
                    velse {
                        vif({ bench.nluOk && bench.nluIntent.isNotEmpty() }) {
                            statusBadge({ "connected" }, { "命中 · ${bench.nluIntent}" })
                        }
                        velse {
                            vif({ bench.nluText.isNotEmpty() }) {
                                statusBadge(
                                    { if (bench.nluOk) "idle" else "error" },
                                    { if (bench.nluOk) "未命中" else "失败" },
                                )
                            }
                            velse {
                                statusBadge({ "idle" }, { "空闲" })
                            }
                        }
                    }
                }
                View { attr { height(AdminSpace.md) } }
                groupedCard("识别详情", withDivider = false) {
                    vif({ bench.nluText.isEmpty() }) {
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                color(AdminColors.textTertiary)
                                text("结果将显示在此（右侧配置卡可修改 bert 模型/词表/标签与阈值）")
                            }
                        }
                    }
                    velse {
                        // 官方文本选中容器：长按进入选中态（渲染层画选区），配「复制」按钮
                        View {
                            ref { bench.nluReplyRef = it }
                            attr {
                                selectable(SelectableOption.ENABLE)
                                selectionColor(AdminColors.accent)
                            }
                            event {
                                longPress { p ->
                                    if (p.state == "start") {
                                        bench.startSelection(bench.nluReplyRef, p.x, p.y)
                                    }
                                }
                                selectEnd { bench.nluCopyHint = "已选中，点「复制」拷贝选中内容" }
                            }
                            Text {
                                attr {
                                    fontSize(AdminType.body)
                                    color(if (bench.nluOk) AdminColors.textPrimary else AdminColors.danger)
                                    text(bench.nluResultText())
                                }
                            }
                            // 推理耗时明细：nlu_test 回包携带（本地 TinyBERT 分词/推理分开计时）
                            vif({ bench.nluOk && (bench.nluTokenizeMs > 0 || bench.nluInferMs > 0) }) {
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        marginTop(AdminSpace.xs)
                                        text("分词 ${bench.nluTokenizeMs} ms / 推理 ${bench.nluInferMs} ms")
                                    }
                                }
                            }
                        }
                        View { attr { height(AdminSpace.sm) } }
                        actionRow {
                            secondaryButton("复制") { bench.copyNlu(ctx) }
                            vif({ bench.nluCopyHint.isNotEmpty() }) {
                                View { attr { width(AdminSpace.sm) } }
                                Text {
                                    attr {
                                        fontSize(AdminType.micro)
                                        color(AdminColors.textTertiary)
                                        text(bench.nluCopyHint)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        right = {
            nluConfigCard(form, ctx)
        },
    )
}
