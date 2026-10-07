package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.PageList
import com.tencent.kuikly.core.views.PageListView
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.Switch
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.TextArea
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.views.compose.Button




// ===================== 下拉选择 =====================

/**
 * 下拉选择字段：字段框（当前项 + ▾/▴）+ 官方 AlertDialog 弹层选择。
 *
 * 弹层用官方 AlertDialog（Modal 渲染）：不占布局（不把下方内容推开）、蒙层点击关闭、
 * 自带显隐过渡；customContentView 自定义暗色卡片（标题 + 可滚动选项列表 + 取消）。
 * 响应式约定（⚠️ Kuikly 只对闭包内的 observable 读取做响应式跟踪）：
 * - isOpen：lambda，由调用方持有展开态（放状态类里避免 body 重建丢失）；
 * - options：返回 **ObservableList** 的 lambda + 内部用 vfor 渲染——选项列表本身可变
 *   （如音色列表随性别切换增删），普通 List 在构建期一次性铺开不会随数据更新。
 */
fun ViewContainer<*, *>.dropdownField(
    label: String,
    currentLabel: () -> String,
    options: () -> ObservableList<Pair<String, String>>,
    selectedId: () -> String,
    isOpen: () -> Boolean,
    onToggle: () -> Unit,
    onSelect: (String) -> Unit,
) {
    fieldLabel(label)
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(48f)
            backgroundColor(AdminColors.fieldBg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            borderRadius(AdminShape.radiusSm)
            paddingLeft(AdminSpace.sm)
            paddingRight(AdminSpace.sm)
        }
        event { click { onToggle() } }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.body)
                color(AdminColors.textPrimary)
                text(currentLabel())
            }
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(if (isOpen()) "▴" else "▾")
            }
        }
    }
    // 官方 AlertDialog 弹层（Modal 渲染，KRModalView iOS/macOS Catalyst 均有实现）
    AlertDialog {
        attr {
            showAlert(isOpen())
            inWindow(true)
            customContentView {
                View {
                    attr {
                        width(340f)
                        backgroundColor(AdminColors.cardBg)
                        borderRadius(AdminShape.radiusLg)
                        border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
                        padding(AdminSpace.cardPadding)
                    }
                    Text {
                        attr {
                            fontSize(AdminType.section)
                            fontWeightMedium()
                            color(AdminColors.textPrimary)
                            marginBottom(AdminSpace.sm)
                            text(label)
                        }
                    }
                    Scroller {
                        attr {
                            height(320f)
                            backgroundColor(AdminColors.insetBg)
                            borderRadius(AdminShape.radiusSm)
                        }
                        vfor(options) { opt ->
                            val selected = opt.first == selectedId()
                            View {
                                attr {
                                    height(44f)
                                    flexDirectionRow()
                                    alignItemsCenter()
                                    paddingLeft(AdminSpace.sm)
                                    paddingRight(AdminSpace.md)
                                }
                                event {
                                    click {
                                        onSelect(opt.first)
                                        onToggle()
                                    }
                                }
                                vif({ opt.first == selectedId() }) {
                                    View {
                                        attr {
                                            width(3f)
                                            height(20f)
                                            borderRadius(AdminShape.radiusPill)
                                            backgroundColor(AdminColors.accent)
                                            marginRight(AdminSpace.xs)
                                        }
                                    }
                                }
                                Text {
                                    attr {
                                        flex(1f)
                                        fontSize(AdminType.body)
                                        color(if (opt.first == selectedId()) AdminColors.accentTintText else AdminColors.textPrimary)
                                        text(opt.second)
                                    }
                                }
                            }
                        }
                    }
                    View { attr { height(AdminSpace.sm) } }
                    secondaryButton("取消") { onToggle() }
                }
            }
        }
        event {
            // 蒙层点击关闭（官方注释：用于自定义前景 UI 场景）
            clickBackgroundMask { onToggle() }
            willDismiss { onToggle() }
        }
    }
}

// ===================== 音频波形播放器 =====================

/**
 * 波形播放控件（录音试听 / TTS 回放共用）：
 * [▶/‖ 圆钮] [32 根波形柱（已播 accent 高亮，未播 trackBg）] [时间 0:03/0:12]
 *
 * - "流过"视觉：进度推进时柱子逐根点亮（attr 闭包内读 progress observable，局部更新颜色）。
 * - wave() 返回 64 桶峰值 0~1（原生 getAudioState 采样），渲染时按 32 柱两两取大。
 * - 整块可点（含波形区）切换播放/停止；enabled=false（合成中/无内容）置灰并屏蔽点击。
 * - 进度/波形由调用方轮询原生后写入 observable（TestBenchState），本组件只做纯渲染。
 */
fun ViewContainer<*, *>.waveformPlayer(
    wave: () -> List<Float>,
    progress: () -> Float,
    playing: () -> Boolean,
    durationText: () -> String,
    enabled: () -> Boolean,
    onToggle: () -> Unit,
) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginTop(AdminSpace.fieldGap)
        }
        event { click { if (enabled()) onToggle() } }
        // 播放/停止圆钮（禁用=中性灰，与按钮家族禁用态一致）
        View {
            attr {
                width(44f)
                height(44f)
                borderRadius(AdminShape.radiusPill)
                allCenter()
                backgroundColor(if (enabled()) AdminColors.accent else AdminColors.disabledBg)
            }
            Text {
                attr {
                    fontSize(AdminType.title)
                    color(AdminColors.textOnAccent)
                    text(if (playing()) "‖" else "▶")
                }
            }
        }
        // 波形柱（32 根，等宽；已播放点亮 accent）
        View {
            attr {
                flex(1f)
                height(48f)
                flexDirectionRow()
                alignItemsCenter()
                marginLeft(AdminSpace.sm)
            }
            var i = 0
            while (i < WAVE_BARS) {
                val idx = i
                View {
                    attr {
                        // ⚠️ wave()/progress() 必须在 attr 闭包内调用：
                        // Kuikly 只跟踪闭包内的 observable 读取——若在循环体（构建期）先算出
                        // played/v 再被闭包捕获，进度更新时柱子颜色/高度不会重算（实测「回放
                        // 无时间轴动画」的根因）。
                        val v = maxOf(
                            wave().getOrElse(idx * 2) { 0.1f },
                            wave().getOrElse(idx * 2 + 1) { 0.1f },
                        )
                        val played = progress() * WAVE_BARS > idx
                        flex(1f)
                        height(6f + v * 36f)
                        marginRight(2f)
                        borderRadius(2f)
                        backgroundColor(if (played) AdminColors.accent else AdminColors.trackBg)
                    }
                }
                i++
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                marginLeft(AdminSpace.xs)
                text(durationText())
            }
        }
    }
}

/** 波形柱数量（getAudioState 返回 64 桶，渲染 32 根柱：两桶取一）。 */
private const val WAVE_BARS = 32

// ===================== 状态徽标 =====================

/**
 * 状态徽标：胶囊高 26、paddingH 10、圆点 8 + 文字 micro(13)/600。
 * state: connected/recording/busy/idle/error 对应 accent/danger/warning/中性/danger 色。
 * ⚠️ state/text 必须是 lambda 且只在 attr 闭包内读取——Kuikly 只对闭包内的 observable
 * 读取做响应式跟踪；构建期传入的一次性 String 在状态变化后颜色/文案不会更新。
 */
fun ViewContainer<*, *>.statusBadge(state: () -> String, text: () -> String) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(26f)
            borderRadius(AdminShape.radiusPill)
            paddingLeft(10f)
            paddingRight(10f)
            // 级别颜色在 attr 闭包内按 observable 求值 → 状态变化即时变色
            val s = state()
            when (s) {
                "connected" -> backgroundColor(AdminColors.accentTintBg)
                "recording", "error" -> backgroundColor(AdminColors.dangerTintBg)
                "busy" -> backgroundColor(AdminColors.warningTintBg)
                else -> backgroundColor(AdminColors.insetBg)
            }
        }
        View {
            attr {
                width(8f)
                height(8f)
                borderRadius(AdminShape.radiusPill)
                marginRight(6f)
                backgroundColor(
                    when (state()) {
                        "connected" -> AdminColors.accent
                        "recording", "error" -> AdminColors.danger
                        "busy" -> AdminColors.warning
                        else -> AdminColors.textPlaceholder
                    },
                )
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                fontWeightMedium()
                text(text())
                color(
                    when (state()) {
                        "connected" -> AdminColors.accentTintText
                        "recording", "error" -> AdminColors.dangerTintText
                        "busy" -> AdminColors.warningTintText
                        else -> AdminColors.textTertiary
                    },
                )
            }
        }
    }
}

// ===================== 顶栏 =====================

/**
 * 大标题栏：高 64、flexRow 居中、左右内距 xl(24)、bg windowBg、底部 1px divider。
 * 左 largeTitle(display=34)/500 textPrimary；右可选按钮/徽标（trailing ViewBuilder）。
 * ⚠️ title 是 lambda，须在 Text 的 attr 闭包内读取（如 `{ "配置" }` 或依赖 observable 的
 * `{ when (ctx.selectedSection) ... }`），切 section 时标题才会响应式更新。
 * ⚠️ 顶部避让区（macOS 红黄绿窗口按钮）由 AdminShell 的内容区/侧边栏 paddingTop 控制。
 */
fun ViewContainer<*, *>.largeTitleBar(title: () -> String, trailing: ViewBuilder? = null) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            height(64f)
            paddingLeft(AdminSpace.xl)
            paddingRight(AdminSpace.xl)
            backgroundColor(AdminColors.windowBg)
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.display)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                text(title())
            }
        }
        if (trailing != null) {
            trailing()
        }
    }
    View {
        attr {
            height(1f)
            backgroundColor(AdminColors.divider)
        }
    }
}
