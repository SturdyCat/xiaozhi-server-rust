// tuya.official：涂鸦设备控制策略 + 云签名链路（阶段三 S3c）。
//
// 新架构下 server 只做系统底座，重构前的「涂鸦设备控制策略」功能以插件形式提供。
// 执行模型遵循平台铁律：宿主派发事件 → 插件同步返回 Vec<Effect>（纯决策 + 效果）→
// 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON。
// 数据面（PCM、模型推理、设备协议、HTTP/SSE）一律留在原生宿主，插件不持有 I/O 句柄。
//
// S3c 云签名链路：签名/请求构造/回包解析是纯协议语义，与宿主共用同一份
// `server/src/device/tuya/protocol.rs`（`#[path]` 单源）；凭证经本插件配置命名空间
// 读取（`config` 权限，`access_secret` 为 secret 字段），HTTP 由宿主代发并按步
// 回灌 `http.responses`。生产同步编排仍由持有凭证的 native `CloudClient` 承担。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

// 与宿主 server/src/device/tuya/cloud.rs 共用的纯协议语义（单源，唯一权威）。
// 插件只消费签名/token 解析；其余单源条目在 guest 侧闲置属预期（allow(dead_code)）。
#[allow(dead_code)]
#[path = "../../../server/src/device/tuya/protocol.rs"]
mod protocol;

// S3f：LAN 0x55AA 帧编解码（本插件域单源；宿主侧参考实现是外部 crate rustuya，
// plugins/wasm/tests/tuya_lan.rs 用 rustuya 本尊生成黄金帧逐字节对拍）。
mod lan_codec;
// S3j/S3o：0x6699（v3.5）帧编解码 + v3.5 控制命令帧（S3o 从 lan_codec 拆出）。
mod lan_frame_6699;
// S3i：v3.4/v3.5 TCP 会话层状态机（三步握手 + 会话密钥派生，纯函数）。
mod lan_session;
// LAN 域 AES-128-ECB 原语（编解码与会话编排共用，S3m 起含解密去填充）。
mod lan_crypto;
// S3n：TCP 流式帧切分（粘包/半包重组）+ Established 数据面帧解密。
mod lan_stream;
// S3q：DP 载荷解析单源（与宿主 server/src/device/tuya/dps.rs 共用同一份源码）。
#[path = "../../../server/src/device/tuya/dps.rs"]
mod dps;
// S3q：设备主动上报（DP 状态）接线——DP 缓存 / 在线语义 / 事件面。
mod lan_status;
// LAN 域事件处理器（S3f 起从本文件迁出，见 lan_events 模块注释）。
mod lan_events;
// S3p：会话状态 kv（编排/建链/心跳/重连共用）。
mod lan_state;
// S3p：自动重连与指数退避策略。
mod lan_reconnect;
// S3m/S3o：建链与握手首帧（首连与重连共用）。
mod lan_connect;
// S3n/S3p：心跳节拍 + timer 分发（心跳 / 重连退避）。
mod lan_heartbeat;
// S3m/S3n/S3o：会话步进（握手回包 / 数据面 / 关闭收敛）。
mod lan_orchestrate;

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, types};

/// 插件 id：必须与上传请求 id、manifest.toml 登记项完全一致。
const PLUGIN_ID: &str = "tuya.official";
/// 结果通道前缀：宿主按 `<prefix>:<req_id>` 读回本插件的决策。
const RESULT_PREFIX: &str = "tuya-result";
/// 云签名链路结果通道前缀（S3c：token 获取）。
const CLOUD_TOKEN_PREFIX: &str = "tuya-cloud-token";
/// 设备清单分页结果通道前缀（S3d：devices 目录拉取）。
const CLOUD_DEVICES_PREFIX: &str = "tuya-devices";
/// LAN 域结果通道前缀由 lan_events 持有（S3f 起随处理器迁出）。

struct Component;

/// 从开放形状载荷中提取 req_id（宿主为每次调用生成，结果必须写回同一 id）。
fn req_id_of(payload: &str) -> String {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("req_id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// 解析载荷；非法 JSON 属于业务性错误（不 panic、不降级）。
fn payload_of(payload: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(payload).map_err(|e| format!("载荷非法：{e}"))
}

/// 统一结果效果：把有界决策 JSON 写进 `<prefix>:<req_id>`。
fn result(value: &Value, req_id: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{}:{}", RESULT_PREFIX, req_id),
        value: value.to_string().into_bytes(),
    })
}

/// 要求 req_id 存在；缺失时终止本次决策，宿主显式失败（fail-closed）。
fn checked_req_id(payload: &str) -> Result<String, String> {
    let rid = req_id_of(payload);
    if rid.is_empty() {
        return Err("缺少 req_id".to_string());
    }
    Ok(rid)
}

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new(PLUGIN_ID, "涂鸦设备控制策略")
            .expect("静态构建：id/title 已内联校验")
            .version("0.4.0")
            .description("connector.tuya 扩展点：候选选择策略 + 云签名链路（config 凭证）")
            .permission("kv")
            .expect("kv 权限")
            .permission("config")
            .expect("config 权限（凭证与区域读取）")
            .permission("net")
            .expect("net 权限（S3m 会话编排出站 TCP）")
            .permission("timer")
            .expect("timer 权限（S3n 心跳节拍 interval）")
            .permission("pub")
            .expect("pub 权限（S3q 设备上报状态事件）")
            .subscribe("io.tcp.*", None)
            .expect("订阅 io.tcp.*（出站连接生命周期回灌）")
            .permission("http-out:openapi.tuyacn.com")
            .expect("http-out（cn 区）")
            .permission("http-out:openapi.tuyaus.com")
            .expect("http-out（us 区）")
            .permission("http-out:openapi.tuyaeu.com")
            .expect("http-out（eu 区）")
            .permission("http-out:openapi.tuyain.com")
            .expect("http-out（in 区）")
            .implements("connector.tuya")
            .expect("扩展点")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry(
                "nav.main",
                "tuya-policy",
                "涂鸦策略",
                "插件",
                114,
                "settings",
            )
            .expect("静态构建")
            .config_field(
                FieldSpec::bool("prefer_online", "优先在线设备")
                    .hint("true")
                    .help("候选选择偏好")
                    .build(),
            )
            .expect("静态构建")
            .config_field(
                FieldSpec::enum_("region", "区域", &["cn", "us", "eu", "in"])
                    .hint("cn")
                    .help("OpenAPI 区域（决定签名请求发往哪个 host）")
                    .build(),
            )
            .expect("静态构建")
            .config_field(
                FieldSpec::str("access_id", "Access ID")
                    .hint("")
                    .help("涂鸦云 access_id；缺省时云签名链路显式报错")
                    .build(),
            )
            .expect("静态构建")
            .config_field(
                FieldSpec::str("access_secret", "Access Secret")
                    .hint("")
                    .secret()
                    .help("涂鸦云 access_secret；只写不读，config-secret 读取并审计")
                    .build(),
            )
            .expect("静态构建")
            .action("ping", "连通性自检", false, "返回插件 id、扩展点与契约版本")
            .expect("静态构建")
            .build()
    }

    fn handle_event(ev: types::Event) -> Result<Vec<types::Effect>, String> {
        let event = ev.topic.clone();
        // S3m：io.tcp.* 的 payload 是原始字节（宿主只搬运字节），correlation-id
        // = connection-id；其余 topic 维持 UTF-8 JSON 载荷面。
        match event.as_str() {
            "io.tcp.data" => {
                return lan_orchestrate::on_tcp_data(&ev.payload, ev.correlation_id.as_deref());
            }
            "io.tcp.closed" => {
                return lan_orchestrate::on_tcp_closed(ev.correlation_id.as_deref());
            }
            // 连接建立通知：编排状态在 connect 时已写 WaitResp，此处忽略。
            "io.tcp.opened" => return Ok(vec![]),
            // S3n：心跳节拍（payload 为 UTF-8 JSON：{"timer_id","kind"}）。
            "host.timer.fired" => {
                let payload = String::from_utf8_lossy(&ev.payload).to_string();
                return lan_heartbeat::on_timer_fired(&payload);
            }
            _ => {}
        }
        let payload = String::from_utf8_lossy(&ev.payload).to_string();
        host::log("debug", &format!("{} event={}", PLUGIN_ID, event));
        match event.as_str() {
            "connector.tuya.prepare" => {
                let rid = checked_req_id(&payload)?;
                let v = payload_of(&payload)?;
                let action = v.get("action").and_then(Value::as_str).unwrap_or("on");
                // 只选候选索引：local_key / DP / LAN 会话仍由原生连接器持有。
                let index = v
                    .get("candidates")
                    .and_then(Value::as_array)
                    .and_then(|arr| {
                        arr.iter().position(|c| {
                            c.get("online").and_then(Value::as_bool).unwrap_or(false)
                                && c.get("has_switch_dp")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(0);
                Ok(vec![result(
                    &json!({
                        "candidate_index": index,
                        "action": action,
                        "route": "native",
                    }),
                    &rid,
                )])
            }
            // 会话观察者挂钩：接受但不干预，避免产生未知事件噪音。
            e if e.starts_with("hook.") => Ok(vec![]),
            // S3c：云签名链路第一步——读自身 config 凭证，构造官方 v1.0 签名的
            // token 请求交宿主代发；回包经 http.responses 回灌后解析落 kv。
            "connector.tuya.cloud-token" => {
                let rid = checked_req_id(&payload)?;
                cloud_token_start(&rid, &payload)
            }
            // S3d：设备清单分页——payload 带 token 直接取页；不带则自动先取
            // token（回灌链 2 步，宿主回灌深度上限 4 之内）。
            "connector.tuya.cloud-devices" => {
                let rid = checked_req_id(&payload)?;
                cloud_devices_start(&rid, &payload)
            }
            // S3f/S3g：LAN 帧编解码（纯函数证据面；rustuya 对拍见测试）。
            "connector.tuya.lan-pack" => {
                let rid = checked_req_id(&payload)?;
                lan_events::lan_pack(&rid, &payload)
            }
            "connector.tuya.lan-parse" => {
                let rid = checked_req_id(&payload)?;
                lan_events::lan_parse(&rid, &payload)
            }
            // S3h/S3i：控制命令帧（version = "3.3" 缺省 / "3.4" 会话密钥模式）。
            "connector.tuya.lan-command" => {
                let rid = checked_req_id(&payload)?;
                lan_events::lan_command(&rid, &payload)
            }
            // S3i：v3.4 TCP 会话协商三步握手（start 帧 / verify+finish）。
            "connector.tuya.lan-session-start" => {
                let rid = checked_req_id(&payload)?;
                lan_events::lan_session_start(&rid, &payload)
            }
            "connector.tuya.lan-session-finish" => {
                let rid = checked_req_id(&payload)?;
                lan_events::lan_session_finish(&rid, &payload)
            }
            // S3m：会话编排入口（连接 + 三步握手状态机，传输走宿主 tcp-connect）。
            "connector.tuya.lan-connect" => {
                let rid = checked_req_id(&payload)?;
                lan_connect::lan_connect(&rid, &payload)
            }
            // 宿主 HTTP 回灌（数组 [{id,status,body,error}]，按步批量）。
            "http.responses" => on_http_responses(&payload),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_service(
        service: String,
        method: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        // 未声明服务的插件一律显式拒绝（宿主 call-service 会拿到这条消息）
        Err(format!(
            "{PLUGIN_ID} 未提供服务 {service}.{method}（{} 字节载荷）",
            payload.len()
        ))
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            "ping" => Ok(format!(
                "插件={} 扩展点=connector.tuya 契约=0.1 参数={}",
                PLUGIN_ID, payload
            )),
            other => Err(format!("未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        let rid = req_id_of(r#"{"req_id":"selftest"}"#);
        let parsed = payload_of(r#"{"req_id":"selftest","v":1}"#)?;
        // 单源协议黄金向量：guest 与宿主逐字节一致（S3b/S3c 证据，见
        // plugins/wasm/tests/protocol.rs 与 device::tuya::cloud_tests）。
        let sign = protocol::official_sign(
            "asec",
            "aid",
            "",
            "1700000000000",
            "GET",
            "/v1.0/token?grant_type=1",
            "",
        );
        let token_ok = protocol::parse_token_ok(
            r#"{"success":true,"result":{"access_token":"t","expire_time":3600}}"#,
        )
        .map(|(t, e)| t == "t" && e == 3600)
        .unwrap_or(false);
        // S3f：LAN 帧编解码回环（pack → parse → seqno/cmd/payload 一致）。
        let frame = lan_codec::pack_frame_55aa(7, 9, b"{\"dp\":1}", None).unwrap_or_default();
        let parsed_frame = lan_codec::parse_frame_55aa_strip(&frame, None);
        let lan_ok = parsed_frame
            .map(|p| {
                p.seqno == 7
                    && p.cmd == 9
                    && p.payload == b"{\"dp\":1}".to_vec()
                    && p.retcode.is_none()
            })
            .unwrap_or(false);
        // S3i：会话状态机回环——start 帧可解出 nonce 密文；自造设备回包 →
        // finish 验证通过，派生密钥满足 XOR 定义（local ⊕ remote 循环）。
        let local_key = [7u8; 16];
        let local_nonce = [3u8; 16];
        let remote_nonce = [11u8; 16];
        let session_ok = (|| -> Result<bool, String> {
            let start = lan_session::session_start_frame(1, &local_key, &local_nonce)?;
            let expect_payload = lan_crypto::aes_ecb_encrypt_pkcs7(&local_key, &local_nonce)?;
            let parsed = lan_codec::parse_frame_55aa_exact(&start, Some(&local_key))?;
            if parsed.cmd != lan_session::SESS_KEY_NEG_START || parsed.payload != expect_payload {
                return Ok(false);
            }
            // 自造回包：remote_nonce || HMAC(local_key, local_nonce)。
            let mut resp = remote_nonce.to_vec();
            resp.extend_from_slice(&lan_session::hmac_sha256(&local_key, &local_nonce)?);
            let finish = lan_session::session_finish(2, &local_key, &local_nonce, &resp)?;
            let want_raw: Vec<u8> = local_nonce
                .iter()
                .zip(remote_nonce.iter())
                .map(|(a, b)| a ^ b)
                .collect();
            Ok(finish.remote_nonce == remote_nonce.to_vec()
                && finish.session_key_blob
                    == lan_crypto::aes_ecb_encrypt(&local_key, &want_raw, false)?)
        })()
        .unwrap_or(false);
        Ok(vec![
            types::TestResult {
                name: "req-id-parse".to_string(),
                ok: rid == "selftest" && parsed.get("v").and_then(Value::as_i64) == Some(1),
                detail: format!("req_id={} 结果通道={}:<req_id>", rid, RESULT_PREFIX),
            },
            types::TestResult {
                name: "protocol-single-source-golden".to_string(),
                ok: sign == "42AD3292C57626B9ED5B83615A5F3735CBFA4C9BC9F9F512C8A22A24BDE0C0E2"
                    && token_ok,
                detail: "单源签名与 token 解析黄金向量".to_string(),
            },
            types::TestResult {
                name: "lan-frame-roundtrip".to_string(),
                ok: lan_ok,
                detail: "LAN 0x55AA 帧编解码回环（CRC 变体）".to_string(),
            },
            types::TestResult {
                name: "lan-session-roundtrip".to_string(),
                ok: session_ok,
                detail: "v3.4 会话协商回环（start 帧 + finish 验证 + 密钥派生）".to_string(),
            },
        ])
    }
}

xiaozhi_plugin_sdk::export!(Component);

/// 读取插件配置（`config` 权限）；凭证缺失即业务性错误（fail-closed，不编造）。
fn read_credentials() -> Result<(String, String, String), String> {
    let region = host::config_get("region")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "cn".to_string());
    let access_id = host::config_get("access_id")
        .filter(|s| !s.trim().is_empty())
        .ok_or("tuya.official 未配置 access_id（管理台插件配置）")?;
    let secret = host::config_secret("access_secret")
        .filter(|s| !s.trim().is_empty())
        .ok_or("tuya.official 未配置 access_secret")?;
    Ok((
        region.trim().to_string(),
        access_id.trim().to_string(),
        secret.trim().to_string(),
    ))
}

/// S3c 第一步：官方 v1.0 签名的 token 请求（单源协议），交宿主代发。
fn cloud_token_start(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    let (region, access_id, secret) = read_credentials()?;
    let v = payload_of(payload)?;
    // t 由调用方传（测试确定性）；缺省用宿主墙上时钟（毫秒）。
    let t = v
        .get("t")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| host::now_ms().to_string());
    let req = protocol::build_token_request(&region, &access_id, &secret, &t);
    Ok(vec![http_request_effect(
        &format!("cloud-token|{rid}"),
        req,
    )])
}

/// S3d 第一步：设备清单分页。payload 带 token 直接取页；不带则自动先取 token
/// （`t` / 游标编码进 request-id 随波携带，回灌链 ≤2 步）。
fn cloud_devices_start(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    let (region, access_id, secret) = read_credentials()?;
    let v = payload_of(payload)?;
    let t = v
        .get("t")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| host::now_ms().to_string());
    let cursor = v
        .get("last_row_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let explicit_token = v
        .get("token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match explicit_token {
        Some(token) => Ok(vec![devices_page_effect(
            rid, &region, &access_id, &secret, &token, &t, &cursor,
        )]),
        None => {
            let req = protocol::build_token_request(&region, &access_id, &secret, &t);
            Ok(vec![http_request_effect(
                &format!("devices-token|{rid}|{t}|{cursor}"),
                req,
            )])
        }
    }
}

/// 签发设备清单页请求（官方签名 + 字典序游标 query，见单源协议）。
fn devices_page_effect(
    rid: &str,
    region: &str,
    access_id: &str,
    secret: &str,
    token: &str,
    t: &str,
    cursor: &str,
) -> types::Effect {
    let req = protocol::build_devices_request(region, access_id, secret, token, t, cursor);
    http_request_effect(&format!("devices-page|{rid}"), req)
}

/// 宿主代发的统一效果构造（GET / 空 body；签名与发送共用同一串 URL）。
fn http_request_effect(id: &str, req: protocol::CloudRequest) -> types::Effect {
    types::Effect::HttpRequest(types::EffectHttpRequest {
        request_id: id.to_string(),
        method: req.method.to_string(),
        url: req.url,
        headers: req.headers,
        body: Vec::new(),
        timeout_ms: 0,
    })
}

/// 取一步回包：先 error 后 body（宿主失败形状 error 非空）。
fn step_result(feedback: &Value) -> Result<String, String> {
    let error = feedback
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if let Some(e) = error {
        return Err(e.to_string());
    }
    Ok(feedback
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// 设备链失败也落结果通道（ok:false），调用方以结果存在性判定完成与否。
fn devices_error_effect(rid: &str, error: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{CLOUD_DEVICES_PREFIX}:{rid}"),
        value: json!({"ok": false, "error": error})
            .to_string()
            .into_bytes(),
    })
}

/// S3c/S3d 回灌：按 request-id 前缀分派三段链路。
///
/// - `cloud-token|<rid>` → 解析 token 落 kv（S3c 结果通道）；
/// - `devices-token|<rid>|<t>|<cursor>` → token 到达 → 立即签发设备清单页请求；
/// - `devices-page|<rid>` → 页形状解析（单源）→ `tuya-devices:<rid>` 结果通道。
///
/// 设备行是原始 JSON（含 local_key，属凭证材料）：留在插件隔离 kv，宿主同步
/// 编排接管前不外溢；单页 ≤50 台（PAGE_SIZE），不受 kv 64KB 上限威胁。
fn on_http_responses(payload: &str) -> Result<Vec<types::Effect>, String> {
    let batch: Vec<Value> =
        serde_json::from_str(payload).map_err(|e| format!("回灌载荷非法：{e}"))?;
    let mut effects = Vec::new();
    for f in &batch {
        let id = f.get("id").and_then(Value::as_str).unwrap_or_default();
        if let Some(rid) = id.strip_prefix("cloud-token|") {
            let value = match step_result(f).and_then(|b| protocol::parse_token_ok(&b)) {
                Ok((token, expire)) => json!({
                    "ok": true,
                    "access_token": token,
                    "expires_in": expire,
                    "route": "plugin",
                }),
                Err(e) => json!({"ok": false, "error": e}),
            };
            effects.push(types::Effect::KvPut(types::EffectKvPut {
                key: format!("{CLOUD_TOKEN_PREFIX}:{rid}"),
                value: value.to_string().into_bytes(),
            }));
        } else if let Some(rest) = id.strip_prefix("devices-token|") {
            let mut parts = rest.splitn(3, '|');
            let (Some(rid), t, cursor) = (
                parts.next(),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            ) else {
                continue;
            };
            let effect = match step_result(f)
                .and_then(|b| protocol::parse_token_ok(&b).map(|(token, _)| token))
            {
                Ok(token) => {
                    let (region, access_id, secret) = read_credentials()?;
                    devices_page_effect(rid, &region, &access_id, &secret, &token, t, cursor)
                }
                Err(e) => devices_error_effect(rid, &e),
            };
            effects.push(effect);
        } else if let Some(rid) = id.strip_prefix("devices-page|") {
            let value = match step_result(f).and_then(|b| protocol::parse_devices_page_ok(&b)) {
                Ok((devices, has_more, cursor)) => json!({
                    "ok": true,
                    "count": devices.len(),
                    "has_more": has_more,
                    "last_row_key": cursor,
                    "devices": devices,
                    "route": "plugin",
                }),
                Err(e) => json!({"ok": false, "error": e}),
            };
            effects.push(types::Effect::KvPut(types::EffectKvPut {
                key: format!("{CLOUD_DEVICES_PREFIX}:{rid}"),
                value: value.to_string().into_bytes(),
            }));
        }
    }
    Ok(effects)
}
