package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text

/**
 * 「唤醒」配置卡（`[wakeup]`）：**唤醒 → LLM 之间**的唤醒问候语。
 *
 * 打开后，设备唤醒词命中（固件立刻上报 `listen state=detect`）时服务器马上说一句预制话术
 * （默认「我在」），**跳过 LLM 与正常回复**——唤醒是全系统最高频的动作，为一句固定的话付一次
 * 大模型调用没有道理。成品音频按音色三级复用：内存缓存（跨会话）→ 磁盘（跨重启）→
 * 真合成一次后自动落盘；稳态下每次唤醒**零 LLM、零 TTS**。
 *
 * 独立文件的原因：`AGENTS.md` §5.10 文件规模约定（`ConfigCards.kt` 已接近上限），
 * 且这张卡的字段/文案自成一组（与 `ConfigCommandCard.kt` 同思路）。
 */
fun ViewContainer<*, *>.wakeupConfigCard(form: ConfigFormState) {
    val wake = form.wake
    groupedCard("唤醒问候语（唤醒 → LLM）") {
        // 整段档位提示（保存后新会话生效）：来自 /api/config/schema，服务端为唯一权威
        vif({ form.schema.sectionNote("wakeup").isNotEmpty() }) {
            textNote(form.schema.sectionNote("wakeup"), micro = true)
        }
        switchRow("enabled（唤醒后立刻回一句，跳过 LLM）", wake.enabled == "true") {
            wake.enabled = if (wake.enabled == "true") "false" else "true"
            form.dirty = true
        }
        Text {
            attr {
                fontSize(AdminType.caption)
                color(AdminColors.textTertiary)
                text("设备唤醒词命中后固件立刻上报 listen state=detect（需固件 CONFIG_SEND_WAKE_WORD_DATA=y），服务器收到即说问候语，不再调用大模型、也不进入正常回复。默认关闭。")
            }
        }
        vif({ wake.enabled == "true" }) {
            labeledField(
                "greeting（问候语）",
                { wake.greeting },
                { wake.greeting = it; form.dirty = true },
                "我在",
            )
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("改这句话会换缓存键，因此会自动重新合成一次并落盘；之后唤醒直接复用。")
                }
            }
            // 问候语是下行音频，播放期间上行音频在缓冲区排队——太长会推迟用户第一句话。
            vif({ wake.greeting.trim().length > 10 }) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.warningTintText)
                        text("⚠️ 问候语偏长（建议 ≤ 10 字）：它播放期间用户的第一句话会在缓冲区排队，等它说完才被处理。服务端对超过 30 字的问候语会直接拦下并提示。")
                    }
                }
            }
            switchRow("persist（落盘复用，重启后仍免合成）", wake.persist == "true") {
                wake.persist = if (wake.persist == "true") "false" else "true"
                form.dirty = true
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("把成品音频（已编码的下行 Opus 帧，约 2KB）写到配置文件同级的 hot-audio/ 目录；容器重建后仍在挂载卷上。换音色会自动重新合成并另存一份，不会串用旧音色。")
                }
            }
            // ⚠️ 跨段矛盾必须显式说出来：复用经内存缓存承载成品帧，缓存关掉时落盘会静默失效。
            vif({ wake.persist == "true" && (form.ttsCacheEntries.trim().toIntOrNull() ?: 256) <= 0 }) {
                Text {
                    attr {
                        fontSize(AdminType.caption)
                        color(AdminColors.warningTintText)
                        text("⚠️ 当前 [tts].cache_entries = 0（TTS 结果缓存已关闭），音频**无法落盘**：每次唤醒都会重新合成。请到「TTS」页把「结果缓存条目数」设为大于 0（推荐 256），或把这里的 persist 关掉。保存时服务端也会拦下并提示。")
                    }
                }
            }
        }
        vif({ wake.enabled != "true" }) {
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textTertiary)
                    text("未启用：唤醒后静默等待用户说话（行为与加本插件之前一致）。")
                }
            }
        }
    }
}
