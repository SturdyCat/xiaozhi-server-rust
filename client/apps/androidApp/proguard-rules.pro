# Kuikly 核心入口类与方法（核心库已通过 consumer-rules 自动提供，这里补充业务侧保留项）
-keep class com.xiaozhi.admin.** { *; }

# 保留 @Page 注解标记的页面类（Kuikly 运行时按页面名反射查找）
-keep @com.tencent.kuikly.core.annotations.Page class * { *; }
