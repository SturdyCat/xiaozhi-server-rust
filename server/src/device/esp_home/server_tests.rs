//! ESPHome 桥协议往返测试（自 `server.rs` 外置：测试一律独立文件，逻辑文件 ≤600 行）。
//!
//! 协议往返测试：不依赖 ASR/TTS 引擎、不依赖模型文件，直接验证桥是否真的「说对了话」。
//!
//! 用裸 TCP + 手写帧（`0x00` 标记 + 变长载荷长度 + 类型字节 + protobuf 载荷）扮演
//! ESPHome 客户端，覆盖 `handle_conn` 的四条真实路径：Hello 握手（由 crate 托管）→
//! DeviceInfo → 实体列表 → 订阅快照 → 状态变化回推。

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 消息类型号（取自 crate `parser::proto_message_mappings!`）。
pub(super) const HELLO_REQUEST: u8 = 1;
pub(super) const HELLO_RESPONSE: u8 = 2;
pub(super) const DEVICE_INFO_REQUEST: u8 = 9;
pub(super) const DEVICE_INFO_RESPONSE: u8 = 10;
pub(super) const LIST_ENTITIES_REQUEST: u8 = 11;
pub(super) const LIST_ENTITIES_BINARY_SENSOR: u8 = 12;
pub(super) const LIST_ENTITIES_DONE: u8 = 19;
pub(super) const SUBSCRIBE_STATES_REQUEST: u8 = 20;
pub(super) const BINARY_SENSOR_STATE: u8 = 21;
pub(super) const LIST_ENTITIES_NUMBER: u8 = 49;
pub(super) const NUMBER_STATE: u8 = 50;
/// 客户端 → 服务端的「设定数值」命令（HA 拖滑块）。
pub(super) const NUMBER_COMMAND: u8 = 51;

fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// 明文帧：`0x00` + 载荷长度（不含类型字节）+ 类型 + 载荷。
pub(super) fn frame(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    encode_varint(payload.len() as u64, &mut out);
    out.push(msg_type);
    out.extend_from_slice(payload);
    out
}

/// 读一帧，返回（类型，载荷）。
pub(super) async fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut marker = [0u8; 1];
    stream.read_exact(&mut marker).await.unwrap();
    assert_eq!(marker[0], 0, "本端应回明文帧（未配 encryption_key）");

    let mut len: u64 = 0;
    let mut shift = 0;
    loop {
        let mut b = [0u8; 1];
        stream.read_exact(&mut b).await.unwrap();
        len |= u64::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }

    // 长度不含类型字节，故多读 1 字节取类型。
    let mut buf = vec![0u8; len as usize + 1];
    stream.read_exact(&mut buf).await.unwrap();
    (buf[0], buf[1..].to_vec())
}

pub(super) fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// 读一个 protobuf varint，返回（值，占用的字节数）。
fn read_varint(buf: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    let mut shift = 0;
    for (i, b) in buf.iter().enumerate() {
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return (value, i + 1);
        }
        shift += 7;
    }
    panic!("varint 未结束");
}

/// 取某个数值字段；**字段缺省返回 0**——protobuf 会省略默认值，`key=0` / `state=false`
/// 在线上就是"不出现"，不能靠"载荷前两字节"判断。
///
/// 同时支持 varint（`state` 等 bool/uint32）与 fixed32（`key`：ESPHome 的
/// `BinarySensorStateResponse.key` 在当前 proto 版本是 `fixed32`，线上 4 字节小端）。
pub(super) fn field_u32(payload: &[u8], want_tag: u8) -> u64 {
    let mut i = 0;
    while i < payload.len() {
        let (key, n) = read_varint(&payload[i..]);
        i += n;
        let tag = (key >> 3) as u8;
        match key & 7 {
            // varint
            0 => {
                let (value, n) = read_varint(&payload[i..]);
                i += n;
                if tag == want_tag {
                    return value;
                }
            }
            // fixed32（小端 4 字节）
            5 => {
                let bytes: [u8; 4] = payload[i..i + 4].try_into().unwrap();
                i += 4;
                if tag == want_tag {
                    return u64::from(u32::from_le_bytes(bytes));
                }
            }
            // length-delimited：跳过（字符串/嵌套消息），本测试只查数值字段
            2 => {
                let (len, n) = read_varint(&payload[i..]);
                i += n + len as usize;
            }
            // fixed64（double/fixed64）：跳过 8 字节
            1 => i += 8,
            other => panic!("未预期的 wire type {other}（tag={tag}）载荷={payload:02X?}"),
        }
    }
    0
}

/// 取某个 `float` 字段（wire type 5，小端 4 字节）；**缺省返回 0.0**——同样因为
/// protobuf 省略默认值，`min_value=0` / `state=0.0` 在线上不出现。
pub(super) fn field_f32(payload: &[u8], want_tag: u8) -> f32 {
    let mut i = 0;
    while i < payload.len() {
        let (key, n) = read_varint(&payload[i..]);
        i += n;
        let tag = (key >> 3) as u8;
        match key & 7 {
            0 => {
                let (_, n) = read_varint(&payload[i..]);
                i += n;
            }
            5 => {
                let bytes: [u8; 4] = payload[i..i + 4].try_into().unwrap();
                i += 4;
                if tag == want_tag {
                    return f32::from_le_bytes(bytes);
                }
            }
            2 => {
                let (len, n) = read_varint(&payload[i..]);
                i += n + len as usize;
            }
            1 => i += 8,
            other => panic!("未预期的 wire type {other}（tag={tag}）载荷={payload:02X?}"),
        }
    }
    0.0
}

/// 一次完整往返：握手 → 设备信息 → 实体定义（5 binary + 1 number）→ 订阅快照 →
/// 二值/数值状态实时回推 → `number` 控制命令（连接保持可用）。
///
/// 连接处理**内联**跑（不经 accept_loop 的 spawn），这样 `handle_conn` 的失败原因
/// 会直接以 `Err` 冒出来，而不是变成客户端侧的 `UnexpectedEof` 哑谜。
#[tokio::test]
async fn plaintext_roundtrip_and_state_push() {
    // `esphome-native-api` 用 `log` 打协议日志（读取循环的终止原因），主程序用 tracing。
    // 这里装一个只在测试期存在的 logger，否则 bridge 内部报错完全静默。
    let _ = env_logger::builder().is_test(true).try_init();

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = EspHomeConfig {
        port: addr.port(),
        mdns: false,
        ..Default::default()
    };

    // 先并发建连 + accept，再把客户端的探测逻辑丢到独立任务里。
    let (connect, accept) = tokio::join!(TcpStream::connect(addr), listener.accept());
    let mut stream = connect.unwrap();
    let (server_stream, _) = accept.unwrap();

    let client = tokio::spawn(async move {
        // 1) Hello：字段号是 client_info=1 / major=2 / minor=3（与 api.proto 一致），
        //    应答由 crate 托管。proto 版本 1.10 与本桥上报的一致。
        let mut hello = vec![0x0a, 0x04];
        hello.extend_from_slice(b"test");
        hello.extend_from_slice(&[0x10, 0x01, 0x18, 0x0a]);
        stream
            .write_all(&frame(HELLO_REQUEST, &hello))
            .await
            .unwrap();
        let (ty, _) = read_frame(&mut stream).await;
        assert_eq!(ty, HELLO_RESPONSE, "期望 HelloResponse");

        // 2) DeviceInfo：应带上配置里的节点名。
        stream
            .write_all(&frame(DEVICE_INFO_REQUEST, &[]))
            .await
            .unwrap();
        let (ty, payload) = read_frame(&mut stream).await;
        assert_eq!(ty, DEVICE_INFO_RESPONSE, "期望 DeviceInfoResponse");
        assert!(contains(&payload, b"xiaozhi-dot2"), "DeviceInfo 应含节点名");

        // 3) 实体列表：5 条 binary_sensor + 1 条 number + Done，object_id 按声明顺序。
        stream
            .write_all(&frame(LIST_ENTITIES_REQUEST, &[]))
            .await
            .unwrap();
        let mut listed: Vec<(u8, Vec<u8>)> = Vec::new();
        loop {
            let (ty, payload) = read_frame(&mut stream).await;
            if ty == LIST_ENTITIES_DONE {
                break;
            }
            listed.push((ty, payload));
        }
        let expected: [(u8, &str); 6] = [
            (LIST_ENTITIES_BINARY_SENSOR, "dot2_online"),
            (LIST_ENTITIES_BINARY_SENSOR, "dot2_listening"),
            (LIST_ENTITIES_BINARY_SENSOR, "dot2_thinking"),
            (LIST_ENTITIES_BINARY_SENSOR, "dot2_speaking"),
            (LIST_ENTITIES_BINARY_SENSOR, "dot2_muted"),
            (LIST_ENTITIES_NUMBER, "dot2_volume"),
        ];
        assert_eq!(listed.len(), expected.len(), "实体数量应与 ENTITIES 一致");
        for (i, (want_ty, object_id)) in expected.iter().enumerate() {
            assert_eq!(listed[i].0, *want_ty, "第 {i} 个实体的类型不符");
            assert!(
                contains(&listed[i].1, object_id.as_bytes()),
                "第 {i} 个实体应含 object_id={object_id}"
            );
        }
        // 音量的类型参数必须是 0..127 / 步长 1，否则 HA 的滑块范围是错的。
        let volume = &listed[5].1;
        assert_eq!(field_f32(volume, 6), 0.0, "min_value=0 属默认值，线上省略");
        assert_eq!(field_f32(volume, 7), 127.0, "max_value 应为 127");
        assert_eq!(field_f32(volume, 8), 1.0, "step 应为 1");

        // 4) 订阅：全量快照 = 5 条 binary + 1 条 number，key 与实体索引一致。
        stream
            .write_all(&frame(SUBSCRIBE_STATES_REQUEST, &[]))
            .await
            .unwrap();
        for i in 0..state::BINARY_COUNT as u64 {
            let (ty, payload) = read_frame(&mut stream).await;
            assert_eq!(ty, BINARY_SENSOR_STATE, "期望 BinarySensorStateResponse");
            assert_eq!(
                field_u32(&payload, 1),
                i,
                "key 应为实体索引 {i}（缺省即 0）"
            );
            assert_eq!(field_u32(&payload, 2), 0, "初始状态应为 false");
        }
        let (ty, payload) = read_frame(&mut stream).await;
        assert_eq!(ty, NUMBER_STATE, "期望 NumberStateResponse");
        assert_eq!(
            field_u32(&payload, 1),
            state::VOLUME as u64,
            "音量 key 应为 VOLUME"
        );
        assert_eq!(field_f32(&payload, 2), 0.0, "初始音量应为 0");

        // 5) 二值状态回推：speaking(key=3) 置 true 应立即推给已订阅的客户端。
        state::set_speaking(true);
        let (ty, payload) = read_frame(&mut stream).await;
        assert_eq!(ty, BINARY_SENSOR_STATE, "期望状态回推");
        assert_eq!(field_u32(&payload, 1), 3, "speaking 的 key 应为 3");
        assert_eq!(field_u32(&payload, 2), 1, "speaking 应被推为 true");
        // 复位也会回推，先吃掉，避免干扰后续断言。
        state::set_speaking(false);
        let _ = read_frame(&mut stream).await;

        // 6) 数值状态回推：音量置 42.5 应以 NumberStateResponse 推给客户端。
        state::set_volume(42.5);
        let (ty, payload) = read_frame(&mut stream).await;
        assert_eq!(ty, NUMBER_STATE, "期望音量回推");
        assert_eq!(
            field_u32(&payload, 1),
            state::VOLUME as u64,
            "音量 key 应为 VOLUME"
        );
        assert_eq!(field_f32(&payload, 2), 42.5, "音量应被推为 42.5");
        state::set_volume(0.0);
        let _ = read_frame(&mut stream).await;

        // 7) 控制命令：发 NumberCommandRequest（key=VOLUME, state=30.0）应被接受且
        //    不中断连接。测试里无设备在线，故下发失败是预期的——这里只验证协议路径
        //    通畅（命令能被解析、连接继续正常应答）。
        let mut cmd = vec![
            0x0d, // key: tag=1, wire type 5 (fixed32)
            state::VOLUME as u8,
            0x00,
            0x00,
            0x00,
            0x15, // state: tag=2, wire type 5 (float)
        ];
        cmd.extend_from_slice(&30.0f32.to_le_bytes());
        stream
            .write_all(&frame(NUMBER_COMMAND, &cmd))
            .await
            .unwrap();
        // 命令不产生回包；用一次 ListEntities 确认连接仍活着。
        stream
            .write_all(&frame(LIST_ENTITIES_REQUEST, &[]))
            .await
            .unwrap();
        let mut count = 0;
        loop {
            let (ty, _) = read_frame(&mut stream).await;
            if ty == LIST_ENTITIES_DONE {
                break;
            }
            count += 1;
        }
        assert_eq!(count, expected.len(), "命令之后连接仍应正常应答");
    });

    // 内联跑连接处理：客户端探测若失败或提前断开，这里立刻返回 Err（含真实原因），
    // 而不是让客户端卡在 `UnexpectedEof` 上看不出所以然。
    let served = handle_conn(server_stream, &cfg, None, state::CatalogPlan::passthrough()).await;
    client.await.expect("客户端探测任务不应 panic");
    served.expect("连接处理应正常结束");
}

/// 命令分派：只有音量实体接受命令，其余 key 一律忽略（不误触发别的动作）。
#[test]
fn number_command_only_targets_volume() {
    assert_eq!(number_command_target(state::VOLUME as u32, 30.4), Some(30));
    assert_eq!(
        number_command_target(state::VOLUME as u32, 127.9),
        Some(127)
    );
    // 其它实体的 key（binary_sensor）与时越界 key 都不应触发动作。
    assert_eq!(number_command_target(state::ONLINE as u32, 50.0), None);
    assert_eq!(number_command_target(999, 50.0), None);
}
