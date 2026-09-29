// HarmonyOS 宿主启动页：承载 Kuikly 页面。
// 实际接入请在 EntryAbility 中初始化 Kuikly 鸿蒙渲染器并打开 shared 中 @Page 对应的页面。
// 参考 Kuikly 官方「鸿蒙工程接入」文档完成渲染器依赖与适配器注册。

import { KuiklyRenderView } from '@kuikly/core-render-ohos';

@Entry
@Component
struct Index {
  @State message: string = 'XiaoZhi Admin'

  build() {
    Column() {
      // Kuikly 页面容器：pageName 对应 shared 中的 @Page("router")
      KuiklyRenderView({ pageName: 'router', pageData: {} })
        .width('100%')
        .height('100%')
    }
    .width('100%')
    .height('100%')
  }
}
