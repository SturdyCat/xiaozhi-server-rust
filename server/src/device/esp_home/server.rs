//! ESPHome Native API 桥：TCP accept + 每连接协议应答 + 状态回推 + 控制面命令分发。
//!
//! ## 为什么不用 `esphomeserver::EspHomeServer`
//!
//! v3.0.0 的 `EspHomeServer::start` 是**空壳**：`ListEntitiesRequest` 里实体列表的循环
//! 被 `// TODO: Handle the different entity types` 注释掉，只回 `ListEntitiesDoneResponse`；
//! 其余消息原样回显，且**从不发状态响应**。用它会「编译通过但什么都暴露不出去」。
//! 这里改用低层 `esphomeapi::EspHomeApi`——它已托管 Hello / Authentication / DeviceInfo /
//! Ping / Disconnect 握手与 noise_psk 加密——并自行应答实体列表 / 订阅 / 状态回推 /
//! 可写实体的命令。协议与加密仍由 crate 托管。
//!
//! ## 连接内状态机
//!
//! 1. `EspHomeApi` 自动答 Hello / Authentication / DeviceInfo / Ping / Disconnect；
//! 2. 本模块答 `ListEntitiesRequest` → 逐条实体定义（`binary_sensor` / `number`）+ Done；
//! 3. 本模块答 `SubscribeStatesRequest` → 全部实体当前状态；
//! 4. 状态变化（`state` 广播）→ 对应实体的状态响应实时回推；
//! 5. `NumberCommandRequest`（HA 拖滑块）→ 经 [`super::actions`] 翻成 EchoMuse 下发。

use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use esphome_native_api::esphomeapi::EspHomeApi;
use esphome_native_api::parser::ProtoMessage;
use esphome_native_api::proto::{
    BinarySensorStateResponse, ListEntitiesBinarySensorResponse, ListEntitiesDoneResponse,
    ListEntitiesNumberResponse, NumberStateResponse,
};

use super::config::EspHomeConfig;
use super::{actions, bluetooth, state};

/// BLE 代理是否生效：桥自己的开关 **且** 设备侧 `[echomuse].ble_proxy` 都为真。
///
/// 单一真相源是 `echomuse.ble_proxy`（设备扫描/连接的总闸），`[esp_home].ble_proxy`
/// 只是允许桥把它宣告出去。二者任一为假就不宣告，避免客户端看到必然失败的能力。
/// （`wasm-session` 引擎不走这里——它与 BLE 代理不兼容，启动期守卫直接拒绝。）
pub(crate) fn ble_proxy_enabled(
    cfg: &EspHomeConfig,
    engines: Option<&std::sync::Arc<crate::runtime::Engines>>,
) -> bool {
    cfg.ble_proxy && engines.is_some_and(|e| e.config.echomuse.ble_proxy)
}

/// 每连接的协议引擎（S3v）：native = `EspHomeApi` 托管（含 `wasm-policy`——协议仍
/// native）；`wasm-session` = 协议面进 esphome.official 插件，由 [`super::session`]
/// 编排（测试直接构造 `WasmSession` 变体注入真实插件宿主，不经 `Engines`）。
enum ConnEngine {
    /// native 协议面。`None` = 协议往返测试（不启动 BLE 代理任务）。
    Native(Option<std::sync::Arc<crate::runtime::Engines>>),
    /// wasm-session：真连接由插件会话编排（真宿主在位由启动期守卫保证）。
    #[cfg(feature = "wasm-plugins")]
    WasmSession(std::sync::Arc<crate::plugins::wasm::WasmHost>),
}

/// 启动桥：绑定 `port` 并无限 accept。`main` 在 `[esp_home].enabled` 时调用。
pub async fn serve(
    cfg: EspHomeConfig,
    engines: std::sync::Arc<crate::runtime::Engines>,
) -> Result<()> {
    let wasm_session = cfg.engine_id() == "wasm-session";

    // 每连接协议引擎（S3v）：wasm-session 在绑定端口**之前**过启动期守卫
    // （缺 feature/宿主/插件、ble_proxy 组合都显式失败——起一个"能力与配置
    // 不符"的桥不如明确失败，不静默降级）。
    #[cfg(feature = "wasm-plugins")]
    let conn_engine = if wasm_session {
        ensure_wasm_session_ready(&cfg, engines.wasm.as_ref())?;
        ConnEngine::WasmSession(engines.wasm.clone().expect("守卫已校验宿主在位"))
    } else {
        ConnEngine::Native(Some(engines.clone()))
    };
    #[cfg(not(feature = "wasm-plugins"))]
    let conn_engine = if wasm_session {
        anyhow::bail!(
            "[esp_home].engine = \"wasm-session\" 需要以 wasm-plugins feature 构建的服务端（当前二进制未编入）"
        );
    } else {
        ConnEngine::Native(Some(engines.clone()))
    };

    // BLE 代理要两个开关同时为真才宣告：桥自己的 `[esp_home].ble_proxy` 与设备侧的
    // `[echomuse].ble_proxy`。后者才是设备扫描/连接的总闸，只开前者会造成
    // 「客户端看到代理能力、点进去却没有设备可用」——§7 不做清单里明令不谎报能力。
    let ble_proxy = ble_proxy_enabled(&cfg, Some(&engines));

    // 实体目录计划：native = 全放行直通；`wasm-policy` / `wasm-session` = 启动期向
    // `connector.esp-home` 策略插件征询（fail-closed：插件不可用桥不启动，不静默
    // 降级为全放行）。在绑定端口**之前**决策。
    let plan = catalog_plan(&cfg, Some(&engines))?;

    // 启动前校验（`build()` 会解码 base64 密钥；非法则直接暴露，不带着坏配置起服务）。
    // wasm-session 引擎不经 native crate，`encryption_key` 不参与协议——跳过。
    if !wasm_session {
        let _ = build_api(&cfg, ble_proxy)?;
    }

    let listener = TcpListener::bind(("0.0.0.0", cfg.port))
        .await
        .map_err(|e| anyhow::anyhow!("绑定 0.0.0.0:{} 失败：{e}", cfg.port))?;
    let exposed = (0..state::ENTITIES.len())
        .filter(|key| plan.is_exposed(*key))
        .count();
    // wasm-session 的加密由插件配置（noise_psk）决定，桥不读插件配置（凭证隔离）。
    let encryption = if wasm_session {
        "插件配置决定（noise_psk）"
    } else if cfg.encryption_key.is_empty() {
        "明文"
    } else {
        "noise_psk"
    };
    tracing::info!(
        "[esp_home] ESPHome Native API 已监听 0.0.0.0:{}（实体 {} 放行 / {}，目录={}，加密={}，mDNS={}，bluetooth_proxy={}）",
        cfg.port,
        exposed,
        state::ENTITIES.len(),
        cfg.engine_id(),
        encryption,
        cfg.mdns,
        if ble_proxy { "已宣告" } else { "未宣告" }
    );

    // 实体状态全部来自既有真相源（registry 会话态 + echomuse 设备遥测），
    // 桥只做只读观测并翻译，不自己推导——避免出现第二份会漂移的状态。
    spawn_state_probe(engines.registry.clone());

    // mDNS 的 api_encryption 宣告只在 native 引擎且配置了密钥时给出；
    // wasm-session 下桥不知道插件配置，如实"不宣告"（连接后由 DeviceInfo
    // 如实上报 api_encryption_supported）。
    if cfg.mdns {
        spawn_mdns(&cfg, !wasm_session && !cfg.encryption_key.is_empty());
    }

    accept_loop(listener, cfg, conn_engine, plan).await
}

/// 构建实体目录计划。
///
/// - `engine = "native"`（默认）：全放行直通，与引入策略前行为逐字节一致；
/// - `engine = "wasm-policy"` / `"wasm-session"`：向 `connector.esp-home` 策略插件
///   征询（同一插件同一决策面，目录口径不因协议引擎分叉）；缺 feature、缺宿主或
///   缺插件都显式报错（桥不启动），与 echomuse/tuya 的策略纪律同口径。
fn catalog_plan(
    cfg: &EspHomeConfig,
    engines: Option<&std::sync::Arc<crate::runtime::Engines>>,
) -> Result<state::CatalogPlan> {
    let engine = cfg.engine_id();
    if !matches!(engine.as_str(), "wasm-policy" | "wasm-session") {
        return Ok(state::CatalogPlan::passthrough());
    }
    #[cfg(not(feature = "wasm-plugins"))]
    {
        let _ = engines;
        anyhow::bail!(
            "[esp_home].engine = \"{engine}\" 需要以 wasm-plugins feature 构建的服务端（当前二进制未编入）"
        )
    }
    #[cfg(feature = "wasm-plugins")]
    {
        let host = engines
            .and_then(|engines| engines.wasm.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("[esp_home].engine = \"{engine}\" 但 WASM 宿主未装载")
            })?;
        let plan = super::policy::esp_home_catalog(&host)
            .map_err(|e| anyhow::anyhow!("ESPHome 目录策略不可用：{e}"))?;
        let exposed = (0..state::ENTITIES.len())
            .filter(|key| plan.is_exposed(*key))
            .count();
        let renamed = (0..state::ENTITIES.len())
            .filter(|key| plan.display_name(*key) != state::ENTITIES[*key].name)
            .count();
        tracing::info!(
            "[esp_home] 目录策略已应用（engine={engine}）：放行 {}/{} 个实体，改名 {renamed} 个",
            exposed,
            state::ENTITIES.len()
        );
        Ok(plan)
    }
}

/// `engine = "wasm-session"` 的启动期守卫（S3v，fail-closed：桥不启动而非静默降级）。
///
/// - WASM 宿主未装载 → 显式报错（缺 `wasm-plugins` feature 的构建在 `serve` 处
///   已先行显式报错，进不到这里）；
/// - `connector.esp-home` 未解析到 `esphome.official`（未装载/未激活，或被探针类
///   实现顶替）→ 显式报错——会话事件只有官方插件会处理；
/// - `ble_proxy = true` 组合 → 显式报错：BLE 消息面未进插件会话子集（§7 不谎报
///   能力），请改回 `native` / `wasm-policy` 或关闭 ble_proxy。
#[cfg(all(feature = "esp_home", feature = "wasm-plugins"))]
fn ensure_wasm_session_ready(
    cfg: &EspHomeConfig,
    wasm: Option<&std::sync::Arc<crate::plugins::wasm::WasmHost>>,
) -> Result<()> {
    use crate::plugins::wasm::Resolution;

    let host = wasm.ok_or_else(|| {
        anyhow::anyhow!("[esp_home].engine = \"wasm-session\" 但 WASM 宿主未装载")
    })?;
    let plugin = match host.resolve(super::session::EXTENSION_POINT) {
        Resolution::Resolved(id) => id,
        other => anyhow::bail!(
            "wasm-session 引擎需要 {} 已装载并激活（{} 解析结果：{other:?}）",
            super::session::SESSION_PLUGIN,
            super::session::EXTENSION_POINT
        ),
    };
    if plugin != super::session::SESSION_PLUGIN {
        anyhow::bail!(
            "wasm-session 引擎要求 {} 由 {} 实现，当前解析到 {plugin}",
            super::session::EXTENSION_POINT,
            super::session::SESSION_PLUGIN
        );
    }
    if cfg.ble_proxy {
        anyhow::bail!(
            "[esp_home].engine = \"wasm-session\" 与 ble_proxy = true 不兼容：\
             BLE 消息面未进插件会话子集，桥不谎报能力。\
             请将 [esp_home].ble_proxy 设为 false，或改用 native / wasm-policy 引擎"
        );
    }
    tracing::info!(
        "[esp_home] wasm-session 引擎就绪：插件={plugin}（加密密钥 noise_psk 由插件配置提供）"
    );
    Ok(())
}

/// accept 循环。**单独抽出**是为了让测试能绑 `127.0.0.1:0`（随机端口）跑真实协议往返，
/// 不必经过 `serve` 的固定端口与 mDNS 广播。
async fn accept_loop(
    listener: TcpListener,
    cfg: EspHomeConfig,
    engine: ConnEngine,
    plan: state::CatalogPlan,
) -> Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let cfg = cfg.clone();
        let engine = match &engine {
            ConnEngine::Native(bt) => ConnEngine::Native(bt.clone()),
            #[cfg(feature = "wasm-plugins")]
            ConnEngine::WasmSession(host) => ConnEngine::WasmSession(std::sync::Arc::clone(host)),
        };
        let plan = plan.clone();
        tokio::spawn(async move {
            let result = match engine {
                ConnEngine::Native(bt) => handle_conn(stream, &cfg, bt, plan).await,
                #[cfg(feature = "wasm-plugins")]
                ConnEngine::WasmSession(host) => {
                    super::session::handle_conn(stream, &cfg, host, plan).await
                }
            };
            if let Err(e) = result {
                tracing::debug!("[esp_home] 连接 {peer} 结束：{e:#}");
            }
        });
    }
}

/// 构造 ESPHome API 句柄（server 角色：本端在 accept 到的流上应答对端请求）。
///
/// `build()` 是 fallible 的：唯一失败原因是 `encryption_key` 不是合法 base64，
/// 故把它当作「启动前配置校验」复用。
fn build_api(cfg: &EspHomeConfig, ble_proxy: bool) -> Result<EspHomeApi> {
    let base = EspHomeApi::builder()
        .name(cfg.device_name.clone())
        .friendly_name(cfg.friendly_name.clone())
        .model(cfg.model.clone())
        .manufacturer(cfg.manufacturer.clone())
        .suggested_area(cfg.suggested_area.clone())
        .api_version_major(1)
        .api_version_minor(10)
        // 客户端在 api >= 1.9 时只读 feature_flags（legacy 版本号仅旧客户端兜底）。
        // 能力位必须与 `bluetooth.rs` 实际实现一致，否则客户端会摆出必然失败的操作。
        .legacy_bluetooth_proxy_version(if ble_proxy {
            bluetooth::LEGACY_VERSION
        } else {
            0
        })
        .bluetooth_proxy_feature_flags(if ble_proxy {
            bluetooth::FEATURE_FLAGS
        } else {
            0
        });
    let built = if cfg.encryption_key.is_empty() {
        base.build()
    } else {
        base.encryption_key(cfg.encryption_key.clone()).build()
    };
    built.map_err(|e| {
        anyhow::anyhow!("ESPHome API 配置非法（多为 encryption_key 非合法 base64）：{e}")
    })
}

/// 单条客户端连接：取发送端/接收端，循环应答协议请求并回推状态变化。
///
/// `bt` 为 `None` 时（协议往返测试）不启动 BLE 代理任务，BLE 相关消息直接丢弃——
/// 测试只验证实体面，不依赖引擎，也不该被 BLE 代理拖进来。
async fn handle_conn(
    stream: TcpStream,
    cfg: &EspHomeConfig,
    bt: Option<std::sync::Arc<crate::runtime::Engines>>,
    plan: state::CatalogPlan,
) -> Result<()> {
    let peer = stream.peer_addr().ok();

    // ⚠️ 必须先等到对端首字节，再交棒给 `EspHomeApi::start`。
    // 该 crate 的 `start()` 是按「客户端主动连服务端」写的：握手第一步就对流做
    // `fill_buf()` 并要求「已有数据」，否则立刻以 `HandshakeError::NoData` 断开。
    // 我们是服务端，accept 之后客户端可能一个字节都还没发，直接 start 会 100% 断连。
    // `peek` 不消费数据（字节留在内核缓冲，随后 `fill_buf` 照常读到），返回 0 = 对端已断。
    let mut probe = [0u8; 1];
    if stream.peek(&mut probe).await? == 0 {
        tracing::debug!("[esp_home] 连接 {peer:?} 建立即断开（未发任何数据）");
        return Ok(());
    }

    let api = build_api(cfg, ble_proxy_enabled(cfg, bt.as_ref()))?;
    let conn = api
        .start(stream)
        .await
        .map_err(|e| anyhow::anyhow!("协议握手失败：{e}"))?;
    tracing::debug!("[esp_home] ESPHome 客户端已连接 {peer:?}");

    let tx = conn.sender();
    let mut rx = conn.receiver();
    let mut changes = state::subscribe();

    // BLE 代理跑在**独立任务**里：一次 `connect` 最长等 35s，若在下面的 select 里
    // await，整个连接期间连状态回推都会停摆。这里只做转发。
    let bt_tx = bt.map(|engines| {
        let (bt_tx, bt_rx) = mpsc::unbounded_channel();
        bluetooth::spawn(engines, bt_rx, tx.clone());
        bt_tx
    });

    loop {
        tokio::select! {
            incoming = rx.recv() => match incoming {
                // BLE 消息先截走（必须排在其它分支之前，否则会被末尾的 `Ok(_)` 吞掉）。
                Ok(msg) if bt_tx.is_some() && bluetooth::is_bluetooth(&msg) => {
                    if let Some(bt) = &bt_tx {
                        if bt.send(msg).is_err() {
                            tracing::debug!("[esp_home] BLE 代理任务已退出，丢弃消息");
                        }
                    }
                }
                Ok(ProtoMessage::ListEntitiesRequest(_)) => {
                    tracing::debug!("[esp_home] ListEntitiesRequest");
                    send_entity_list(&tx, &plan).await;
                }
                Ok(ProtoMessage::SubscribeStatesRequest(_)) => {
                    tracing::debug!("[esp_home] SubscribeStatesRequest");
                    send_all_states(&tx, &plan).await;
                }
                // HA 拖音量滑块：命令经翻译缝下发到设备，**不回推**（设备会自己上报
                // `volume_state`，探针读真实值再推；提前推会把滑块弹回旧值）。
                Ok(ProtoMessage::NumberCommandRequest(req)) => {
                    // 目录策略未放行的实体不进 ListEntities，客户端不应发来命令；
                    // 收到也只忽略（与未知 key 同路径），不产生动作。
                    if !plan.is_exposed(req.key as usize) {
                        tracing::debug!(
                            "[esp_home] 忽略未放行实体的 number 命令 key={}",
                            req.key
                        );
                        continue;
                    }
                    match number_command_target(req.key, req.state) {
                        Some(level) => {
                            tracing::debug!("[esp_home] 音量命令 {:.2} → 档位 {level}", req.state);
                            actions::set_volume(level);
                        }
                        None => tracing::debug!("[esp_home] 忽略未知 number 命令 key={}", req.key),
                    }
                }
                // 其余消息（日志订阅、时间请求等）当前忽略。
                Ok(_) => {}
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!("[esp_home] 客户端消息滞后，跳过 {n} 条");
                }
                Err(RecvError::Closed) => break,
            },
            change = changes.recv() => match change {
                Ok(key) => send_current(&tx, &plan, key).await,
                // 丢事件就全量补一次，保证客户端最终一致。
                Err(RecvError::Lagged(_)) => send_all_states(&tx, &plan).await,
                Err(RecvError::Closed) => {}
            },
        }
    }
    tracing::debug!("[esp_home] ESPHome 客户端断开 {peer:?}");
    Ok(())
}

/// 应答 `ListEntitiesRequest`：逐条实体定义（按种类选消息类型）+ `ListEntitiesDoneResponse`。
///
/// 只发目录计划放行的实体（未放行的不进列表——HA 侧就不会创建，语义干净）；
/// 显示名经目录计划（策略覆盖优先，回落静态名）。**key 恒为 [`state::ENTITIES`] 索引**，
/// 不因放行过滤而重排，状态回推与命令分派的 key 语义不变。
async fn send_entity_list(tx: &mpsc::Sender<ProtoMessage>, plan: &state::CatalogPlan) {
    for (key, spec) in state::ENTITIES.iter().enumerate() {
        if !plan.is_exposed(key) {
            continue;
        }
        let name = plan.display_name(key);
        let msg = match spec.kind {
            state::EntityKind::Binary { device_class } => {
                ProtoMessage::ListEntitiesBinarySensorResponse(ListEntitiesBinarySensorResponse {
                    object_id: spec.object_id.to_string(),
                    key: key as u32,
                    name: name.to_string(),
                    device_class: device_class.to_string(),
                    ..Default::default()
                })
            }
            state::EntityKind::Number {
                min,
                max,
                step,
                unit,
                mode,
            } => ProtoMessage::ListEntitiesNumberResponse(ListEntitiesNumberResponse {
                object_id: spec.object_id.to_string(),
                key: key as u32,
                name: name.to_string(),
                min_value: min,
                max_value: max,
                step,
                unit_of_measurement: unit.to_string(),
                mode,
                ..Default::default()
            }),
        };
        if tx.send(msg).await.is_err() {
            return;
        }
    }
    let _ = tx
        .send(ProtoMessage::ListEntitiesDoneResponse(
            ListEntitiesDoneResponse {},
        ))
        .await;
}

/// 按实体种类回推它的当前状态；目录计划未放行的实体不发（客户端没建过这个实体）。
async fn send_current(tx: &mpsc::Sender<ProtoMessage>, plan: &state::CatalogPlan, key: usize) {
    if !plan.is_exposed(key) {
        return;
    }
    match state::ENTITIES.get(key).map(|spec| spec.kind) {
        Some(state::EntityKind::Binary { .. }) => {
            let on = state::binary_snapshot().get(key).copied().unwrap_or(false);
            send_binary_state(tx, key as u32, on).await;
        }
        Some(state::EntityKind::Number { .. }) => {
            send_number_state(tx, key as u32, state::number_value(key)).await;
        }
        None => {}
    }
}

/// 发一个 `binary_sensor` 的当前状态。
async fn send_binary_state(tx: &mpsc::Sender<ProtoMessage>, key: u32, on: bool) {
    let resp = BinarySensorStateResponse {
        key,
        state: on,
        missing_state: false,
        device_id: 0,
    };
    let _ = tx.send(ProtoMessage::BinarySensorStateResponse(resp)).await;
}

/// 发一个 `number` 的当前值。
async fn send_number_state(tx: &mpsc::Sender<ProtoMessage>, key: u32, value: f32) {
    let resp = NumberStateResponse {
        key,
        state: value,
        missing_state: false,
        device_id: 0,
    };
    let _ = tx.send(ProtoMessage::NumberStateResponse(resp)).await;
}

/// 应答 `SubscribeStatesRequest`：全量状态快照（按 [`state::ENTITIES`] 顺序，仅放行实体）。
async fn send_all_states(tx: &mpsc::Sender<ProtoMessage>, plan: &state::CatalogPlan) {
    for key in 0..state::ENTITIES.len() {
        send_current(tx, plan, key).await;
    }
}

/// `number` 命令的分派规则：**只有音量实体接受命令**，其余返回 `None`（忽略）。
///
/// 抽成纯函数便于测试"命令落到哪个动作"——协议解析（`handle_conn`）与业务翻译
/// （[`actions`]）分离。wasm-session 引擎（[`super::session`]）复用同一规则。
pub(crate) fn number_command_target(key: u32, state: f32) -> Option<u8> {
    (key as usize == state::VOLUME).then(|| actions::quantize_volume(state))
}

/// 每 2s 用既有真相源校正实体状态。
///
/// - **会话态**（聆听 / 思考 / 播报）来自 `device::registry` 的活会话表（`Phase`）；
/// - **设备态**（在线 / 静音 / 音量）来自 `device::echomuse` 的 hub 登记与设备遥测。
///
/// 桥只做只读观测，不自己维护状态表——避免出现第二份会漂移的真相。首轮 `tick()`
/// 立即返回，故启动后无需等 2s 就能得到正确初值。
///
/// 遥测缺失（设备还没上报过音量/静音）时**保持上次值**，不写 0：否则 HA 里会出现
/// "音量突然归零"这种假变化。
fn spawn_state_probe(registry: crate::device::registry::Registry) {
    use crate::device::registry::Phase;

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            tick.tick().await;

            let sessions = registry.sessions();
            state::set_listening(sessions.iter().any(|s| s.phase == Phase::Listening));
            state::set_thinking(sessions.iter().any(|s| s.phase == Phase::Thinking));
            state::set_speaking(sessions.iter().any(|s| s.phase == Phase::Speaking));

            state::set_online(actions::online());
            let telemetry = actions::telemetry();
            if let Some(volume) = telemetry.volume {
                state::set_volume(volume);
            }
            if let Some(muted) = telemetry.muted {
                state::set_muted(muted);
            }
        }
    });
}

/// 广播 mDNS `_esphomelib._tcp`（HA 自动发现）。失败只告警：客户端仍可填 IP:port 手动添加。
///
/// `announce_encryption`：是否在 TXT 里宣告 `api_encryption`。只有 native 引擎且
/// 配置了密钥才为真；wasm-session 引擎的密钥在插件配置里、桥不读插件配置（凭证
/// 隔离），如实不宣告——连接后由 DeviceInfo 的 `api_encryption_supported` 如实上报。
fn spawn_mdns(cfg: &EspHomeConfig, announce_encryption: bool) {
    let device_name = cfg.device_name.clone();
    let friendly_name = cfg.friendly_name.clone();
    let manufacturer = cfg.manufacturer.clone();
    let model = cfg.model.clone();
    let port = cfg.port;
    let mac = fake_mac(&device_name);

    tokio::task::spawn_blocking(move || {
        use mdns_sd::{ServiceDaemon, ServiceInfo};

        let ip = match crate::device::local_ipv4() {
            Some(ip) => ip.to_string(),
            None => {
                tracing::warn!(
                    "[esp_home] 无法探测局域网 IPv4（容器部署常见），mDNS 跳过；\
                     HA 可用 IP:{} 手动添加",
                    port
                );
                return;
            }
        };
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("[esp_home] mDNS daemon 启动失败（可用 IP:port 手动添加）：{e}");
                return;
            }
        };
        let mut props = std::collections::HashMap::new();
        props.insert("version".to_string(), "2026.6.2".to_string());
        props.insert("mac".to_string(), mac);
        props.insert("platform".to_string(), "ESP32".to_string());
        props.insert("board".to_string(), "esp32dev".to_string());
        props.insert("network".to_string(), "wifi".to_string());
        props.insert("friendly_name".to_string(), friendly_name);
        props.insert("manufacturer".to_string(), manufacturer);
        props.insert("model".to_string(), model);
        if announce_encryption {
            props.insert(
                "api_encryption".to_string(),
                "Noise_NNpsk0_25519_ChaChaPoly_SHA256".to_string(),
            );
        }
        let host = format!("{device_name}.local.");
        let info = match ServiceInfo::new(
            "_esphomelib._tcp.local.",
            &device_name,
            &host,
            ip,
            port,
            Some(props),
        ) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("[esp_home] mDNS ServiceInfo 构造失败：{e}");
                return;
            }
        };
        match daemon.register(info) {
            Ok(_) => tracing::info!(
                "[esp_home] mDNS 已广播 _esphomelib._tcp（HA 可自动发现；name={device_name}）"
            ),
            Err(e) => tracing::warn!("[esp_home] mDNS 注册失败（可 IP:port 手动添加）：{e}"),
        }
        // 注册后 daemon 需常驻才有意义：把它的生命周期交给进程（后台线程自持）。
        std::mem::forget(daemon);
    });
}

/// 稳定的伪 MAC（FNV-1a 32 位 + 固定前缀），仅供 mDNS 展示与 wasm-session 身份面，
/// 不参与协议加密。
pub(crate) fn fake_mac(seed: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in seed.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let b = h.to_be_bytes();
    format!(
        "02:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[0],
        b[1],
        b[2],
        b[3],
        (h >> 8) as u8
    )
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

/// S3v：wasm-session 引擎的真 TCP 回归（依赖 WASM 宿主与真实 esphome.wasm 产物）。
#[cfg(all(test, feature = "wasm-plugins"))]
#[path = "session_engine_tests.rs"]
mod session_engine_tests;
