package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.slider.Slider
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.graphicsLayer
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray

/** Official EQ layout: parallel vertical faders, gain above and frequency below. */
@Composable
internal fun EmNativeEqFaders(bands: JSONArray, drafts: EchoMuseConfigDrafts, enabled: Boolean) {
    val frequencies = listOf("125", "250", "500", "1k", "2k", "3.5k", "5.5k", "8k")
    Row(Modifier.fillMaxWidth()) {
        for (i in 0 until bands.length()) {
            val value = (bands.opt(i) as? Number)?.toFloat() ?: continue
            Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("${emNum(value.toDouble(), 0)}", style = Theme.typography.caption, color = Theme.colors.foreground)
                Box(Modifier.fillMaxWidth().height(180.dp), contentAlignment = Alignment.Center) {
                    Slider(value = value.coerceIn(-12f, 12f), enabled = enabled, valueRange = -12f..12f,
                        steps = 23, modifier = Modifier.requiredSize(160.dp, 44.dp).graphicsLayer { rotationZ = -90f },
                        onValueChange = { next ->
                            val updated = JSONArray()
                            for (index in 0 until bands.length()) updated.put(if (index == i) next.toDouble() else bands.opt(index))
                            drafts.eqBandsDraft = updated.toString()
                        })
                }
                Text(if (bands.length() == 8) frequencies[i] else "${i + 1}",
                    style = Theme.typography.caption, color = Theme.colors.mutedForeground)
            }
        }
    }
}
