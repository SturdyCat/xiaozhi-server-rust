# 前端客户端指引

继承[根指引](<../AGENTS.md>)。本文件只放前端平台/构建边界与入口链接；页面和表单维护见 [React 管理台指引](<admin-web/AGENTS.md>)，旧 Kotlin/Kuikly 管理台已 W15 软下线归档（源码保留但不再构建，见 [W15 资产盘点归档](<../docs/plans/admin-web-migration/w15-legacy-assets-research.md>)）。

## 当前管理台（admin-web）

- 当前管理台为 [React 管理台](<admin-web/AGENTS.md>)（`client/admin-web`，Vite + shadcn/ui + Tauri 2），生产静态目录 `client/admin-web/dist`；`[server].admin_dir` / `XIAOZHI_ADMIN_DIR` / Docker `/app/web` 默认指向此处。
- 前端构建：`pnpm --dir client/admin-web install && pnpm --dir client/admin-web build`（产物 `dist/` 与 `dist/em/`）；启用 Tauri 桌面壳需 Rust 工具链与 `src-tauri/` 配置。
- 前端检查：`pnpm --dir client/admin-web format:check` / `typecheck` / `build`；`/em/` 官方页由 `admin-web/scripts/sync-em.mjs` 白名单 4 文件同步进 `dist/em/`。

在 client 目录验证：

```bash
pnpm --dir admin-web typecheck
pnpm --dir admin-web build
```

## 旧端 Kotlin/Kuikly 管理台（W15 软下线归档，保留不构建）

- 旧端源码位于 `shared/`（commonMain / macosArm64Main 管理台与配置页）、`gearuiKit/`（vendored GearUI）、`apps/macosApp/`（Mac Catalyst 原生桥与语音测试台）、`apps/h5App/em/`（官方 `/em/` 页源目录，13 文件保留）。
- W15 起旧端不再参与生产构建；其维护说明与契约数字均保留为史料，不作现行实现依据（[W15 资产盘点归档](<../docs/plans/admin-web-migration/w15-legacy-assets-research.md>)）。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 下级入口

- [React 管理台](<admin-web/AGENTS.md>)：当前管理台开发环境与维护入口。
- [旧端 Kotlin 管理台（归档）](<shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)：W15 前跨端页面、表单与配置契约，已归档。
- [旧端 macOS 宿主（归档）](<apps/macosApp/AGENTS.md>)：Mac Catalyst 原生桥与语音测试台，已归档。
