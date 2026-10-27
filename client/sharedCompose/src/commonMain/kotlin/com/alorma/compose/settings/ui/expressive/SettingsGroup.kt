package com.alorma.compose.settings.ui.expressive

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.ColumnScope
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.material3.ExperimentalMaterial3ExpressiveApi
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.SemanticsPropertyReceiver
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.tencent.kuikly.compose.ui.unit.dp
import com.alorma.compose.settings.ui.core.LocalSettingsGroupEnabled

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsGroup(
  modifier: Modifier = Modifier,
  enabled: Boolean = true,
  contentPadding: PaddingValues = PaddingValues(0.dp),
  verticalArrangement: Arrangement.Vertical = Arrangement.spacedBy(8.dp),
  title: @Composable (() -> Unit)? = null,
  semanticProperties: (SemanticsPropertyReceiver.() -> Unit) = {},
  content: @Composable ColumnScope.() -> Unit,
) {
  Column(
    modifier =
      Modifier
        .fillMaxWidth()
        .semantics(properties = semanticProperties)
        .then(modifier)
        .padding(contentPadding),
    verticalArrangement = verticalArrangement,
  ) {
    if (title != null) {
      SettingsGroupTitle(title)
    }
    CompositionLocalProvider(LocalSettingsGroupEnabled provides enabled) {
      content()
    }
  }
}

@Composable
internal fun SettingsGroupTitle(title: @Composable () -> Unit) {
  Box(
    modifier =
      Modifier
        .fillMaxWidth()
        .padding(vertical = 8.dp, horizontal = 16.dp),
    contentAlignment = Alignment.CenterStart,
  ) {
    title()
  }
}
