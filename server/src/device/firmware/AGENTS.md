# OTA 固件托管使用

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

## 6. OTA 固件托管（多型号 / 多功能）

设备开机 POST `/api/ota` 检查版本，服务端按设备上报的 **功能 + 型号** 匹配托管固件，命中且版本更新才下发下载地址（设备自升级）。

- **寻址维度**：`功能(project) × 型号(model)`，可选 `板型(board)` 收窄。功能 = `esp_app_desc_t.project_name`，型号 = 芯片（`esp32c3`/`esp32s3`…），板型 = `board.type`。
- **匹配规则**：功能与型号**必须完全相同**；同型号优先板型精确匹配，否则回落"该型号通用"。**不同型号/功能的包绝不互发**；没有匹配就不下发。
- **元数据自动识别**：ESP-IDF 镜像自带元数据，上传时自动解析 型号/功能/版本/IDF 版本/编译时间，无需手填。
- **存储布局**：`<XIAOZHI_FIRMWARE_DIR>/<project>/<model>/<board|_>/firmware.bin + meta.json`（默认 `/data/firmware`）。同一 key 上传即覆盖，不留历史版本。

| 端点 | 语义 |
|---|---|
| `POST /api/ota/firmware` | 上传 app bin（`expected_token` 非空需 Bearer；可选 `?project=&model=&board=&version=` 覆盖） |
| `GET /api/ota/firmware` | 托管清单（管理页数据源） |
| `GET /api/ota/firmware/latest?project=&model=&board=` | 设备下载（**不鉴权**：设备无法带 Authorization 头） |
| `POST /api/ota/firmware/delete` / `DELETE /api/ota/firmware` | 删除条目 |

> 旧版单目录固件（`firmware.bin` + `latest.json`）会被列为 `_legacy`：可删可看，但**不参与匹配**——请按 功能+型号 重新上传。

---

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
