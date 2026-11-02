package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.ui.Alignment

/** Single shell-level entry shared by the full admin and standalone config page. */
@Composable
fun AdminInterfaceModeControl(form: ConfigFormState) {
    val pager = LocalAdminPagerOrThrow()
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        for (mode in AdminInterfaceMode.entries) {
            val selected = form.interfaceMode == mode
            Button(
                text = mode.label,
                size = com.gearui.components.button.ButtonSize.SMALL,
                onClick = { form.setInterfaceMode(pager, mode) },
                type = if (selected) ButtonType.FILL else ButtonType.OUTLINE,
                theme = if (selected) ButtonTheme.PRIMARY else ButtonTheme.DEFAULT,
            )
        }
    }
}
