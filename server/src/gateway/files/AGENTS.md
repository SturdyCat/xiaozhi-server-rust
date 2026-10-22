# 数据目录文件管理

[files](<../files.rs>) 提供 `/api/files/*`：管理台"文件"页与脚本对**服务端数据目录**的通用文件维护（列表/建目录/上传/下载/删除），也是 EchoMuse 自定义唤醒词模型的上传通道（`oww-models/`，流程见[根文档](<../../../../docs/echomuse.md>)"自定义唤醒词"）。

- 存储根**复用** [echomuse registry 的 `data_dir`](<../../device/echomuse/registry.rs>)（`[memory].db_path` 父目录，默认 `/data`），不另立配置项、不复制回退逻辑；上传上限与固件托管同值（64MB）。
- 端点（契约冻结，Kuikly 客户端按此实现）：`GET /api/files?path=`（列表，目录在前按名排序，`entries` 含 `name/dir/size/modified/protected`）、`POST /api/files/mkdir`（body `{"path"}`，父目录一并建、目录已存在幂等、同名文件 409）、`POST /api/files/upload?path=&overwrite=`（body 为**裸字节**，自动建父目录，已存在未覆盖 409，空 body 400，超限 413）、`POST /api/files/delete`（body `{"path"}`）与 `DELETE /api/files?path=`（删文件/目录递归，缺失 404）、`GET /api/files/download?path=`（附件下载；目录 400）。
- 错误一律 JSON `{"error"}`：路径非法/以根为目标 400，受保护名 403（文案说明是配置/记忆库文件），缺失 404，冲突 409，超限 413。鉴权与 `/api/config` 同（`auth_ok`：`expected_token` 空 = 不校验，头或 `?token=` 均可）；**`[echomuse].enabled` 不作用于此模块**（通用存储管理，非设备面）。
- `data.db`/`data.db-wal`/`data.db-shm` 受保护（任意层级**最后一段**命中即受保护）：删除与上传覆盖/新建一律 403——那是 SQLite 配置+记忆库。
- 路径校验拒绝 `..`/`.`/空段（`//`、前导 `/`）/反斜杠/NUL，尾斜杠仅作目录习惯写法去掉；解析对最近存在祖先 `canonicalize` 并校验根内（防符号链接外逃）；上传经临时文件 + rename 原子落盘（与固件同款模式）。删除的 POST 别名是 Kuikly NetworkModule 只有 GET/POST 的既有惯例。
- `OWW_MODELS_DIR = "oww-models"` 不预创建（upload/mkdir 按需生成）；服务端只存储与列出，**不做**服务端→设备的模型分发。
- 回归拆两份（共享夹具 `temp_root`/`canonical`/`json` 在前者，兄弟模块经 `super::tests::` 复用）：纯函数矩阵（路径校验/受保护名/防外逃/列表排序/操作契约/路由无冲突）在 [tests.rs](<tests.rs>)；真实 loopback HTTP（`TestContext` 起 axum，**不起**真实 Engines：鉴权门禁、上传→列表→下载→删除回路、覆盖/受保护/穿越矩阵）在 [http_tests.rs](<http_tests.rs>)。逻辑拆分同固件：路径与契约决策在父模块，HTTP 提取在 [http.rs](<http.rs>)。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
