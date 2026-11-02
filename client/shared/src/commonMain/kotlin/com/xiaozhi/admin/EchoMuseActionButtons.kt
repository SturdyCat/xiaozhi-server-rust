package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonShape
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.extension.setProp
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon

/** Official-style circular management action using GearUI's accessible 44dp button target. */
@Composable
internal fun EchoMuseActionButton(
    icon: IconToken,
    contentDescription: String,
    onClick: () -> Unit,
    disabled: Boolean = false,
    loading: Boolean = false,
    destructive: Boolean = false,
) {
    val tint = if (destructive) Theme.colors.destructiveForeground else Theme.colors.foreground
    Button(
        onClick = onClick,
        type = if (destructive) ButtonType.FILL else ButtonType.TEXT,
        theme = if (destructive) ButtonTheme.DANGER else ButtonTheme.DEFAULT,
        shape = ButtonShape.CIRCLE,
        disabled = disabled,
        loading = loading,
        contentDescription = contentDescription,
        iconWidget = {
            GearIcon(token = icon, tint = tint, modifier = Modifier.setProp("touchEnable", 0))
        },
    )
}

@Composable
internal fun EchoMuseRefreshButton(loading: Boolean, onClick: () -> Unit) {
    EchoMuseActionButton(
        icon = IconToken.Refresh,
        contentDescription = "刷新设备列表",
        onClick = onClick,
        disabled = loading,
        loading = loading,
    )
}
