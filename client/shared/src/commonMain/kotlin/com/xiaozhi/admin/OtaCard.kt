package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 「OTA 固件」卡片（配置页的 `ota` tab）：托管清单 + 上传 + 删除。
 *
 * 为什么要有这一页：固件托管此前**只能 curl**（`POST /api/ota/firmware?version=…`，且服务端
 * 只保留"一个最新版"）。多型号/多功能设备接入后，"哪个设备会拿到哪个包"必须可见可管——
 * 发错型号的包会直接变砖。
 *
 * 呈现规则：
 * - **先说清匹配规则**（功能 + 型号，可选板型），再列条目——管理员不必靠猜"为什么这台没升级"；
 * - 每条显示 **会拿到它的设备**（`matchNote`），把"谁会被升级"写死成一句人话；
 * - 型号/版本由**包内元数据自动识别**，手填只是覆盖出口（默认收起）；
 * - 删除要**行内二次确认**（固件是设备变砖级别的资源，一次误点不该生效）；
 * - 上传/删除结果走顶部浮动 toast（与「保存配置」同一条通道）。
 *
 * ⚠️ 渲染函数必须是 `ViewContainer` 扩展，并在目标容器闭包内**非限定**调用（见
 * `ConfigFormState` 文件头的 Kuikly DSL 绑定说明）。
 */
fun ViewContainer<*, *>.otaCard(ota: OtaState, toast: ToastState, ctx: Pager) {
    // ===== 1) 规则说明 + 存储位置 + 清单摘要 =====
    groupedCard("OTA 固件托管") {
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(
                    "服务端按「功能 + 型号」二维托管固件：设备 OTA 时上报 application.name（功能）、" +
                        "chip_model_name（型号）、board.type（板型）；三者命中**且版本更新**才会下发下载地址。" +
                        "不同型号/不同功能的包绝不互发。",
                )
            }
        }
        vif({ ota.summary.isNotEmpty() }) {
            textNote(ota.summary, micro = true)
        }
        vif({ ota.dir.isNotEmpty() }) {
            textNote("存储目录：${ota.dir}", micro = true)
        }
        // 首次加载失败：就地错误 + 可行动指引（不弹 toast——toast 只用于用户动作的结果）
        vif({ ota.loadError.isNotEmpty() }) {
            textNote(ota.loadError + "（确认服务端地址可达后点「刷新清单」）", tone = "error", micro = true)
        }
        actionRow {
            secondaryButton("刷新清单", loading = { ota.loading }, loadingText = "刷新中…") {
                ota.refresh(ctx)
            }
        }
    }

    // ===== 2) 上传 =====
    groupedCard("上传固件包") {
        textNote(
            "选择 idf.py build 产出的 app .bin：功能/型号/版本由**包内元数据自动识别**（ESP-IDF 镜像头" +
                "的 chip_id + esp_app_desc_t），无需手填。同 功能+型号 再次上传即覆盖旧包。",
        )
        vif({ ota.hasPicked }) {
            Text {
                attr {
                    fontSize(AdminType.body)
                    color(AdminColors.accentTintText)
                    marginTop(AdminSpace.xs)
                    text("已选：${ota.pickedName}（${formatBytes(ota.pickedSize)}）")
                }
            }
        }
        vif({ ota.pickError.isNotEmpty() }) {
            textNote(ota.pickError, tone = "warn")
        }
        actionRow {
            secondaryButton("选择 .bin 文件") { ota.pick(ctx) }
            View { attr { width(AdminSpace.sm) } }
            // 三态按钮：未选包不可点 → 上传中… → ✓ 已上传 / ✕ 上传失败（约 2s 回落）
            primaryButton(
                "上传固件",
                enabled = { ota.hasPicked },
                loading = { ota.uploading },
                loadingText = "上传中…",
                result = { ota.uploadResult },
                successText = "✓ 已上传",
                errorText = "✕ 上传失败",
            ) {
                ota.upload(ctx) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            }
        }
        // 手动覆盖默认收起：正常路径（ESP-IDF 包）根本不需要它
        switchRow("手动指定功能/型号/版本（默认自动识别）", ota.manual) {
            ota.manual = !ota.manual
        }
        vif({ ota.manual }) {
            labeledField("功能（project）", { ota.project }, { ota.project = it }, "如 xiaozhi")
            labeledField("型号（model）", { ota.model }, { ota.model = it }, "如 esp32c3 / esp32s3")
            labeledField(
                "板型（board，可选）",
                { ota.board },
                { ota.board = it },
                "如 bread-compact-wifi；留空 = 该型号通用",
            )
            labeledField("版本（version，可选）", { ota.version }, { ota.version = it }, "如 2.0.4；留空 = 用包内版本")
        }
    }

    // ===== 3) 已托管清单 =====
    groupedCard("已托管固件") {
        vif({ ota.loaded && ota.entries.size == 0 }) {
            textNote("尚未托管任何固件：上传后，设备下次 OTA 检查（开机/重连）即可拿到升级。")
        }
        // entries 是 ObservableList（刷新后整表重建），用 vfor 保持响应式
        vfor({ ota.entries }) { row ->
            otaEntryBlock(row, ota, toast, ctx)
        }
    }
}

/** 一个托管条目：标题 + 版本/体积/sha + "谁会被升级" + 删除（行内二次确认）。 */
private fun ViewContainer<*, *>.otaEntryBlock(
    row: OtaEntryRow,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flexDirectionColumn()
            marginTop(AdminSpace.md)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text(row.title)
                }
            }
            // 旧版条目（无型号信息）只在偏离时打标——它是"需要处理"的状态
            if (row.legacy) {
                statusBadge(state = { "busy" }, text = { "旧版" })
            }
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(row.subtitle)
            }
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                marginTop(4f)
                text(row.matchNote)
            }
        }
        actionRow {
            vif({ ota.confirmKey == row.key }) {
                primaryButton("确认删除", danger = { true }) {
                    ota.remove(ctx, row) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                }
                View { attr { width(AdminSpace.sm) } }
                secondaryButton("取消") { ota.confirmKey = "" }
            }
            vif({ ota.confirmKey != row.key }) {
                secondaryButton("删除") { ota.confirmKey = row.key }
            }
        }
        dividerH()
    }
}
