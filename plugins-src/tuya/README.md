# tuya.official — 涂鸦设备控制策略 + 云签名链路

- **插件 id**：`tuya.official`（官方默认实现，`official=true`）
- **版本**：0.4.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`；阶段三收口起移除 legacy `lan-socket` 权限与 LAN 效果事件）
- **扩展点**：`connector.tuya`
- **权限**：`kv` / `config` / `net` / `timer` / `pub` / `http-out:openapi.tuya{cn,us,eu,in}.com`；订阅 `io.tcp.*`（出站连接生命周期回灌）
- **定位**：新架构下 server 只做系统底座；本插件承接重构前「涂鸦设备控制策略」功能的**控制/策略面**，并自 0.3.0 起承接**涂鸦云 OpenAPI 签名链路**（S3c：凭证经插件配置命名空间、签名/解析为与宿主共用的单源协议）。数据面（PCM、模型推理、设备协议、HTTP/SSE 会话）仍由原生宿主持有。

## 功能描述

承接重构前涂鸦 IoT 业务域的控制面。宿主在 `tuya.control` 技能步骤派发 `connector.tuya.prepare`，本插件从候选列表挑第一台在线且带开关 DP 的设备，返回索引与请求动作。`local_key`、DP 编码、UDP/TCP 会话与云同步编排仍由原生连接器持有。

0.3.0 起（阶段三 S3c/S3d/S3e）新增**云面承接**：派发 `connector.tuya.cloud-token` / `connector.tuya.cloud-devices` 时，插件读取自身配置命名空间的 `region` / `access_id` / `access_secret`（secret 经 `config-secret`，读取写审计），用与宿主**同一份**单源协议（`server/src/device/tuya/protocol.rs`，`#[path]` 引入）构造官方 v1.0 签名请求，交宿主代发；回包经 `http.responses` 回灌解析落 kv。**S3e 起 `[tuya].engine = "wasm-policy"` 是生产路径**：native 同步编排（60s 闸门/翻页/SQLite 落库）经本插件云链路取目录（凭证用插件配置命名空间，不读 [tuya] 段）；LAN 数据面与控制策略编排仍 native。凭证永不离开插件配置命名空间。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。宿主派发事件 → 本插件同步返回 `Vec<Effect>`（`kv-put` / `http-request`）→ 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON，逐字段做边界校验后交给原生数据面。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `connector.tuya.prepare` | `{req_id, name, action, candidates[]}` | `tuya-result:<req_id>` | 选候选索引 + 回传请求动作 |
| `connector.tuya.cloud-token` | `{req_id, t?}`（t 缺省用宿主时钟） | `tuya-cloud-token:<req_id>` | 签名 token 请求 → 宿主代发 → 解析回包 `{ok, access_token?, expires_in?, error?, route:"plugin"}` |
| `connector.tuya.cloud-devices` | `{req_id, token?, t?, last_row_key?}` | `tuya-devices:<req_id>` | 单页设备清单拉取：带 token 直接取页；不带则自动先取 token（回灌链 2 步）。回包 `{ok, count, has_more, last_row_key, devices[], route:"plugin"}` |
| `connector.tuya.lan-pack` | `{req_id, seqno, cmd, payload_hex, hmac_key_hex?}` | `tuya-lan:<req_id>` | 编码 0x55AA LAN 帧（CRC 或 HMAC-SHA256 尾，S3f）；回包 `{ok, frame_hex}`。与宿主 rustuya 参考逐字节对拍（`tests/tuya_lan.rs`） |
| `connector.tuya.lan-parse` | `{req_id, data_hex, hmac_key_hex?}` | `tuya-lan:<req_id>` | 解析 0x55AA 帧：头/尾校验 + retcode 启发式（rustuya 同款）；回包 `{ok, seqno, cmd, retcode?, payload_hex?}` / `{ok:false, error}` |
| `connector.tuya.lan-command` | `{req_id, seqno, device_id, cid?, t, command, dps, key_hex, version?}` | `tuya-lan:<req_id>` | 控制命令全链路：`version` 缺省 `"3.3"`（S3h：载荷形状 gwId/devId/uid/t/dps，t 为字符串 → ECB+PKCS7 → 非裸头命令插 "3.3" 头 → 0x55AA **CRC** 帧；`command ∈ {control, update_dps}`）；`"3.4"`（S3i：modern envelope `{protocol:5, t 数值, data:{cid?,ctype?,dps?}}`，Control 生效为 ControlNew(0x0d)，头 "3.4" **进明文整体加密** → 0x55AA **HMAC** 尾帧；`command ∈ {control, update_dps, heartbeat}`，heartbeat 去 uid/t 且无需 t）；`"3.5"`（S3j：载荷臂与 3.4 同款，头 "3.5" 但**不做帧前加密**，加密在帧级 0x6699/GCM，需 `iv_hex`）。回包 `{ok, version, cmd, frame_hex}`。与 rustuya 参考逐字节对拍 |
| `connector.tuya.lan-session-start` | `{req_id, seqno, local_key_hex, nonce_hex, version?, iv_hex?}` | `tuya-lan:<req_id>` | **会话协商第 1 步**（S3i/S3j）：构造 SessKeyNegStart(0x03) 完整帧。`version` 缺省 `"3.4"`（0x55AA HMAC 尾，nonce 经 ECB+PKCS7 加密）；`"3.5"`（0x6699/GCM，nonce 明文进帧，`iv_hex` 12B 必填）。回包 `{ok, version, cmd, frame_hex}`。nonce/IV 由调用方传入（事件面纯函数；guest 可经 `host-core.random` 取熵，编排侧已用） |
| `connector.tuya.lan-connect` | `{req_id, device_id, ip, port, local_key_hex, version?, heartbeat_ms?, reconnect_attempts?, reconnect_base_ms?, reconnect_max_ms?}` | `tuya-lan:<req_id>` + `tuya-lan-state:<device_id>` | **会话编排入口**（S3m/S3n/S3o/S3p，v3.4 / v3.5）：宿主 `tcp-connect` 出站连设备 → guest 发 SessKeyNegStart → 状态 kv 置 `WaitResp`；回包经 `io.tcp.data`（correlation-id = connection-id）步进——**握手阶段也走流式切分**（半包到达只入缓冲、保持 WaitResp）→ 验证派生 → 发 finish → 置 `Established`（blob 落状态 kv）。失败（重复会话/连接失败/握手失败）落结果通道 `ok:false`，状态置 `Failed`；`io.tcp.closed` 置 `Closed`。nonce = 宿主 OS 随机源 16B（S3s `host-core.random`；熵源不可用时回退 `SHA-256(local_key‖device‖now_ms)` 确定性派生）。`heartbeat_ms`（缺省 30s，下限 200ms）驱动单全局 interval timer 心跳；Established 态 `io.tcp.data` 做流式帧切分（粘包/半包重组，缓冲持久化在状态 kv）+ 会话密钥解密 + 心跳应答识别（`last_pong_ms`/`data_frames` 落 kv）。`version` 缺省 `"3.4"`（0x55AA 布局）；`"3.5"`（S3o：0x6699 布局，握手/数据帧每帧 IV 由 `SHA-256(nonce 或会话 blob ‖ seqno ‖ 用途)[..12]` 派生——nonce 现为宿主随机源（不可预测），同连接内 seqno 单调、跨连接 seed 唯一，满足 GCM 不重复 IV）。**自动重连（S3p）**：`reconnect_attempts`（缺省 0 = 不重连，上限 10）+ `reconnect_base_ms`（缺省 1000，下限 50）+ `reconnect_max_ms`（缺省 30000，退避封顶）；连接失败/设备断链 → `Reconnecting`（attempt+1，延迟 = base·2^(attempt-1) 封顶 max，一次性 timeout）→ 到期自动重握手，握手成功 attempt 归零、`reconnects`+1；重试耗尽 → `Failed` |
| `connector.tuya.lan-session-finish` | `{req_id, seqno, local_key_hex, nonce_hex, resp_payload_hex, version?, iv_hex?}` | `tuya-lan:<req_id>` | **会话协商第 2+3 步**（S3i/S3j）：验证设备回包 payload（解密后 48 字节 = remote_nonce‖HMAC(local_key, local_nonce)，验证不过 fail-closed；两代同款）→ 派生会话密钥（local⊕remote 循环 XOR）→ 密钥库 blob（v3.4 = ECB 无填充；v3.5 = GCM 密文前 16 字节，rustuya `[12..28]` 切片怪癖）→ 签发 SessKeyNegFinish(0x05) 帧。回包 `{ok, version, cmd, frame_hex, session_key_hex, remote_nonce_hex}`；后续帧以 blob 作密钥（rustuya `get_cipher` 同口径） |
| （**发布**）`tuya.official.lan-status` | 事件载荷 `{device_id, cmd, count, dps, online, at_ms}` | — | **设备主动上报**（S3q）：Status(0x08) / DpQuery(0x0a) / DpQueryNew(0x10) / UpdateDps(0x12) 的 DP 快照事件（插件发布面，需 `pub` 权限；宿主/未来接管方按 topic 订阅）。DP 解析与 native **单源**（`server/src/device/tuya/dps.rs`，guest 经 `#[path]` 共用） |
| `http.responses`（宿主回灌） | `[{id,status,body,error}]` | 见上 | `id` 前缀分派：`cloud-token\|` / `devices-token\|<rid>\|<t>\|<cursor>` / `devices-page\|<req_id>` |

`req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败（fail-closed，不静默回退）。**凭证未配置时云链路直接业务性失败**（不发 HTTP、不写结果键）。分页是**单页契约**（`PAGE_SIZE=50`，回灌深度上限内的有界链）：调用方以 `has_more` / `last_row_key` 决定是否再次触发，插件不做无界循环。设备行含 `local_key`（凭证材料），只留在插件隔离 kv，宿主同步编排接管前不外溢。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### 配置字段（`[plugins.tuya.official.config]`）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `prefer_online` | bool | true | 候选选择偏好 |
| `region` | enum(cn/us/eu/in) | cn | OpenAPI 区域（决定签名请求发往哪个 host） |
| `access_id` | str | — | 涂鸦云 access_id；缺省时云签名链路显式报错 |
| `access_secret` | str(secret) | — | 只写不读；`config-secret` 读取并审计 |

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `tuya-result:<req_id>` | 结果 JSON | prepare 决策的交付物 |
| `tuya-cloud-token:<req_id>` | 结果 JSON | 云签名链路 token 结果（S3c） |
| `tuya-devices:<req_id>` | 结果 JSON | 设备清单单页结果（S3d） |
| `tuya-lan:<req_id>` | 结果 JSON | LAN 帧编解码 / 控制命令 / 会话协商结果（S3f–S3i，纯函数证据面） |
| `tuya-lan-state:<device_id>` | 连接上下文 JSON | 会话编排状态（S3m/S3o/S3p：conn_id / phase / seqno / nonce / local_key / 会话 blob / attempt / reconnects / 退避诊断；S3q：`dps` 快照 / `online` / `status_count` / `last_status_ms`；凭证材料只留隔离 kv） |

### 边界与限制

- 会话编排（S3m/S3n/S3o/S3p）声明订阅 `io.tcp.*` 并持 `net` + `timer` 权限；TCP 只由宿主搬字节，握手/心跳/分帧/重连状态机全在插件。自动重连为**显式策略**（缺省关闭）：不配置 `reconnect_attempts` 时断链只置 `Closed`；配置后按指数退避自动重连，上限/基数/封顶由 `lan-connect` 载荷决定。生产接管仍以 P8c 真机回归为门槛。状态键 `tuya-lan-state:<device_id>` 含 local_key/nonce/会话 blob（凭证材料，只留插件隔离 kv）。
- `candidate_index` 不得越界、`action` 必须与请求一致；宿主逐项校验。
- 需 `[tuya].engine = "wasm-policy"` 才会派发 `prepare`。
- 云签名链路的错误**分类**（Auth/Api）是宿主业务语义，不进单源协议；插件侧只回业务性错误文本。
- 生产云同步编排仍由持有凭证的 native `CloudClient` 承担；本插件的云签名链路是策略/协议面承接（S3c 证据），未接管同步 worker。
- LAN 会话面（S3i/S3j）是**纯函数状态机**：v3.4/v3.5 三步握手、密钥派生与控制帧都在插件；会话编排（S3m/S3n/S3o/S3p）已接宿主 `tcp-connect` + `io.tcp.*`，连接/握手/心跳/数据面/重连退避均归插件，native LAN worker 仍在（生产接管以 P8c 真机回归为门槛）。
- **设备上报语义（S3q，两处对 native 的修正）**：① native `LanSink` 把任意 JSON 载荷都套 `parse_dps`，心跳回显 `{"devId","gwId"}` 会被当成 DP；插件只接设备上报类命令（Status/DpQuery/DpQueryNew/UpdateDps）。② native 用整表替换 DP 缓存（部分上报会丢先前 DP）；插件按 DP id **增量合并**（上限 128 键）。DP 解析与 native 同源单文件，形状语义由 `dps_tests` 钉死。
- LAN 帧与命令/会话事件的输入解码上限 16KB；结果通道单值受 kv 64KB 上限约束。

- 插件不持有 PCM / 模型 / 设备句柄；LAN `route` 一律 `native`。
- 默认底座构建未编译对应业务 worker 时，本插件只提供策略与签名决策；业务链路仍需带 `native-plugins` 的镜像。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝（fail-closed）。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/tuya.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/tuya.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/tuya.official/register` 与 `POST /api/plugins/tuya.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
