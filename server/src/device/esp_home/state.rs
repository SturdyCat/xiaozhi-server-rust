//! ESPHome 桥的实体表与状态（只读 `binary_sensor` + 可写 `number`）。
//!
//! 状态是**进程级单例**：`server` 每条连接订阅一次变化；业务侧（设备/会话管理）通过
//! `set_*` 写入，桥把变化实时推给所有已订阅的 ESPHome 客户端。
//!
//! 状态真相源不在本模块：
//! - 会话态（聆听/思考/播报）来自 `device::registry` 的活会话表（`Phase`）；
//! - 设备态（在线/静音/音量）来自 `device::echomuse` 的 hub 登记与设备遥测；
//!
//! 桥只做只读观测与翻译，不自己推导，避免出现第二份会漂移的真相。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

use tokio::sync::broadcast;

/// 实体的种类——**决定暴露给 ESPHome 的消息类型**，也决定它是否可写。
#[derive(Clone, Copy)]
pub enum EntityKind {
    /// `binary_sensor`：只读布尔（HA 里不可点）。
    Binary { device_class: &'static str },
    /// `number`：可写数值（HA 里是输入框/滑块，命令回传给桥）。
    Number {
        min: f32,
        max: f32,
        step: f32,
        unit: &'static str,
        /// ESPHome `NumberMode`：0=AUTO / 1=BOX / 2=SLIDER。
        mode: i32,
    },
}

/// 一个实体的静态定义。**在 [`ENTITIES`] 中的索引即 ESPHome 的 `key`**。
#[derive(Clone, Copy)]
pub struct EntitySpec {
    /// ESPHome `object_id`（实体唯一标识，HA 据此生成 entity_id）。
    pub object_id: &'static str,
    /// 显示名。
    pub name: &'static str,
    /// 种类与类型专属参数。
    pub kind: EntityKind,
}

/// 只读二元传感器的数量（key 落在 `0..BINARY_COUNT`）。
pub const BINARY_COUNT: usize = 5;
/// 可写数值实体的数量（key 从 `BINARY_COUNT` 起）。
pub const NUMBER_COUNT: usize = 1;

/// 暴露的实体表。**顺序即 key**；增删须同步 `BINARY_COUNT` / `NUMBER_COUNT` 与索引常量。
///
/// 不用「已唤醒」实体：registry 的 `Phase` 没有唤醒态（唤醒后立刻进 `Listening`），
/// 硬造一个恒为 false 的实体会在 HA 里变成噪声。
///
/// 「静音」也**只做成只读传感器**：EchoMuse 协议有 `mute_state`（设备上报）但**没有
/// `mute_set`**（无下发路径），做成可写 `switch` 会变成"点了没反应"的假控件。
pub const ENTITIES: [EntitySpec; BINARY_COUNT + NUMBER_COUNT] = [
    EntitySpec {
        object_id: "dot2_online",
        name: "在线",
        kind: EntityKind::Binary {
            device_class: "connectivity",
        },
    },
    EntitySpec {
        object_id: "dot2_listening",
        name: "聆听中",
        kind: EntityKind::Binary {
            device_class: "running",
        },
    },
    EntitySpec {
        object_id: "dot2_thinking",
        name: "思考中",
        kind: EntityKind::Binary {
            device_class: "running",
        },
    },
    EntitySpec {
        object_id: "dot2_speaking",
        name: "播报中",
        kind: EntityKind::Binary {
            device_class: "running",
        },
    },
    EntitySpec {
        object_id: "dot2_muted",
        name: "静音",
        kind: EntityKind::Binary {
            device_class: "sound",
        },
    },
    EntitySpec {
        object_id: "dot2_volume",
        name: "音量",
        // 0..127 是设备的原始 tinymix 扬声器档位（上游 `volumeMax=127`），**不是百分比**。
        kind: EntityKind::Number {
            min: 0.0,
            max: 127.0,
            step: 1.0,
            unit: "",
            mode: 1,
        },
    },
];

pub const ONLINE: usize = 0;
pub const LISTENING: usize = 1;
pub const THINKING: usize = 2;
pub const SPEAKING: usize = 3;
pub const MUTED: usize = 4;
/// 音量实体 key（同时也是 `ENTITIES` 索引）。
pub const VOLUME: usize = BINARY_COUNT;

struct State {
    binary: [AtomicBool; BINARY_COUNT],
    /// 数值实体的 `f32::to_bits()`（`AtomicU32` 无浮点原子类型）。
    numbers: [AtomicU32; NUMBER_COUNT],
    /// 广播「哪个实体变了」（实体 key）；无订阅者时发送失败被忽略。
    tx: broadcast::Sender<usize>,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| {
        // 初始全 false / 0：真实值由 `server::spawn_state_probe` 从注册表与设备遥测校正
        //（首轮 tick 立即执行，不需要等一个周期）。不预设 true，避免桥刚起时"谎报在线"。
        let binary = std::array::from_fn(|_| AtomicBool::new(false));
        let numbers = std::array::from_fn(|_| AtomicU32::new(0f32.to_bits()));
        let (tx, _rx) = broadcast::channel(32);
        State {
            binary,
            numbers,
            tx,
        }
    })
}

/// 订阅状态变化（值为实体 key）。桥的每条连接持有一个。
pub fn subscribe() -> broadcast::Receiver<usize> {
    state().tx.subscribe()
}

/// 二元传感器当前快照（按 `0..BINARY_COUNT` 顺序）。
pub fn binary_snapshot() -> [bool; BINARY_COUNT] {
    let st = state();
    std::array::from_fn(|i| st.binary[i].load(Ordering::Relaxed))
}

/// 数值实体的当前值；key 非法（不在数值区间）时返回 0。
pub fn number_value(key: usize) -> f32 {
    let st = state();
    st.numbers
        .get(key.wrapping_sub(BINARY_COUNT))
        .map(|slot| f32::from_bits(slot.load(Ordering::Relaxed)))
        .unwrap_or(0.0)
}

/// 写入二元传感器状态；**值真变化才广播**（避免同一状态反复刷客户端）。
pub fn set(key: usize, value: bool) {
    let st = state();
    if let Some(slot) = st.binary.get(key) {
        if slot.swap(value, Ordering::Relaxed) != value {
            let _ = st.tx.send(key);
        }
    }
}

/// 写入数值实体；同样只在真变化时广播。
pub fn set_number(key: usize, value: f32) {
    let st = state();
    let Some(slot) = st.numbers.get(key.wrapping_sub(BINARY_COUNT)) else {
        return;
    };
    let bits = value.to_bits();
    if slot.swap(bits, Ordering::Relaxed) != bits {
        let _ = st.tx.send(key);
    }
}

pub fn set_online(value: bool) {
    set(ONLINE, value);
}
pub fn set_listening(value: bool) {
    set(LISTENING, value);
}
pub fn set_thinking(value: bool) {
    set(THINKING, value);
}
pub fn set_speaking(value: bool) {
    set(SPEAKING, value);
}
pub fn set_muted(value: bool) {
    set(MUTED, value);
}
pub fn set_volume(value: f32) {
    set_number(VOLUME, value);
}
