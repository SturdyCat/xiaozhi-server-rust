package com.xiaozhi.admin

import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.attr.Color
import com.tencent.kuikly.core.base.attr.allCenter
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.pager.Pager

/**
 * 管理后台根页面（router）。
 * 通过 URL 参数 ?page_name=xxx 或原生路由打开其他 @Page 页面。
 */
@Page("router")
class AdminApp : Pager() {
    override fun body(): ViewBuilder {
        return {
            attr {
                allCenter()
                backgroundColor(Color.WHITE)
            }

            View {
                attr {
                    allCenter()
                    flexDirectionColumn()
                }

                Text {
                    attr {
                        fontSize(20f)
                        color(Color(0xFF222222L))
                        text("XiaoZhi Admin")
                    }
                }
                Text {
                    attr {
                        marginLeft(0f)
                        marginTop(8f)
                        fontSize(14f)
                        color(Color(0xFF888888L))
                        text("Kuikly multiplatform · client")
                    }
                }
            }
        }
    }
}
