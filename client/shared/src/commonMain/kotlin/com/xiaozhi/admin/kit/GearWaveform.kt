package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.gearui.components.slider.Slider
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.Canvas
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.foundation.shape.CircleShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.geometry.CornerRadius
import com.tencent.kuikly.compose.ui.geometry.Offset
import com.tencent.kuikly.compose.ui.geometry.Size
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon

/**
 * 音频波形播放器（录音试听 / TTS 回放共用）—— Compose 版。
 *
 * 旧实现：`[▶/‖ 圆钮] [32 根波形柱] [时间 0:03/0:12]`，柱子"流过"式逐根点亮。
 * 新版：`[▶/‖ 圆钮] [Canvas 32 根柱] [时间]` + 下方一条 GearUI `Slider` 供**拖动定位**。
 *
 * ## 两处必须保住的语义
 *
 * 1. **柱数与取样规则不变**：原生 `getAudioState` 返回 64 桶，渲染 32 根柱，**两桶取大**
 *    （`max(b[2i], b[2i+1])`）—— 旧注释记过：两桶取一而不是取平均，才能保住爆音的可见性。
 * 2. **柱高公式不变**：`6 + v * 36`（px），圆角 2，柱间 2 —— 版式与迁移前逐像素一致。
 *
 * 新增的 `Slider` 不替代波形，只补"可拖动定位"（旧版只能整块点击切换播放/停止）。
 * 拖动回调 [onSeek] 为 null 时 Slider 只读（仅显示进度），保持与旧版一致的行为。
 *
 * @param wave         64 桶峰值（0~1），长度不足时按 0.1 兜底（与旧版 `getOrElse` 默认值一致）
 * @param progress     播放进度 0~1
 * @param playing      是否正在播放（决定圆钮显示 ▶ 还是 ‖）
 * @param durationText 右侧时间文本，如 `0:03/0:12`
 * @param enabled      false 时整块置灰并屏蔽点击（合成中 / 无内容）
 * @param onToggle     点击圆钮或波形区：播放/停止切换
 * @param onSeek       Slider 拖动定位；null = 不提供拖动
 */
@Composable
fun GearWaveform(
    wave: List<Float>,
    progress: Float,
    playing: Boolean,
    durationText: String,
    enabled: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    onSeek: ((Float) -> Unit)? = null,
) {
    val accent = Theme.colors.primary
    val track = Theme.colors.muted
    val disabledBg = Theme.colors.muted
    val onAccent = Theme.colors.primaryForeground

    Column(modifier.fillMaxWidth().padding(top = GearUiSpace.fieldGap)) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(enabled = enabled) { onToggle() },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // 播放/停止圆钮（禁用 = 中性灰，与按钮家族禁用态一致）
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(if (enabled) accent else disabledBg),
                contentAlignment = Alignment.Center,
            ) {
                GearIcon(
                    token = if (playing) IconToken.Stop else IconToken.Play,
                    size = 20.dp,
                    tint = onAccent,
                )
            }

            // 波形区：32 根柱，已播放段用 accent
            Canvas(
                modifier = Modifier
                    .padding(start = GearUiSpace.sm)
                    .weight(1f)
                    .height(48.dp),
            ) {
                val bars = WAVE_BARS
                val slot = this.size.width / bars
                val barWidth = (slot - BAR_GAP).coerceAtLeast(1f)
                val midY = this.size.height / 2f
                for (index in 0 until bars) {
                    val value = maxOf(
                        wave.getOrElse(index * 2) { FALLBACK_PEAK },
                        wave.getOrElse(index * 2 + 1) { FALLBACK_PEAK },
                    )
                    val barHeight = 6f + value * 36f
                    val played = progress * bars > index
                    drawRoundRect(
                        color = if (played) accent else track,
                        topLeft = Offset(
                            x = index * slot,
                            y = midY - barHeight / 2f,
                        ),
                        size = Size(barWidth, barHeight),
                        cornerRadius = CornerRadius(2f, 2f),
                    )
                }
            }

            Text(
                text = durationText,
                modifier = Modifier.padding(start = GearUiSpace.xs),
                style = Theme.typography.caption,
                color = Theme.colors.mutedForeground,
            )
        }

        if (onSeek != null) {
            Slider(
                value = progress.coerceIn(0f, 1f),
                onValueChange = onSeek,
                enabled = enabled,
                valueRange = 0f..1f,
            )
        }
    }
}

/** 波形柱数量（原生返回 64 桶，渲染 32 根柱：两桶取大）。 */
private const val WAVE_BARS = 32

/** 柱间缝隙（px）。 */
private const val BAR_GAP = 2f

/** 波形数据缺失时的兜底峰值（与旧实现 `getOrElse { 0.1f }` 同值）。 */
private const val FALLBACK_PEAK = 0.1f

/** 供测试断言"两桶取大"规则的纯函数（与绘制逻辑同一公式）。 */
internal fun waveBarValue(wave: List<Float>, index: Int): Float = maxOf(
    wave.getOrElse(index * 2) { FALLBACK_PEAK },
    wave.getOrElse(index * 2 + 1) { FALLBACK_PEAK },
)

/** 柱高（px）：`6 + v * 36`，与旧实现同式。 */
internal fun waveBarHeight(value: Float): Float = 6f + value * 36f

/** 第 [index] 根柱是否已播放（`progress * 32 > index`，与旧实现同式）。 */
internal fun waveBarPlayed(progress: Float, index: Int): Boolean = progress * WAVE_BARS > index
