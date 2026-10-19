//! GET 响应的浮点噪声清理：`Config` 的浮点配置项全是 `f32`，而
//! `serde_json::to_value` 产出的 `Value::Number` 内部只能存 `f64` ——
//! `0.7f32` 加宽成 `0.699999988079071` 后原样发给管理台，表单回填就出现巨长小数。
//! （存储层无此问题：`serde_json::to_string(cfg)` 直接序列化时 f32 走最短表示。）
//!
//! 解法：对回包 Value 树做一遍值级清理 —— 仅当某 f64 **恰好是某个 f32 加宽而来**
//!（`f == (f as f32) as f64`）时，改用该 f32 的最短十进制表示重建 Number。
//! 客户端原生 f64（如 `0.7000000001`）不满足该判据，原样保留，无损。

use serde_json::Value;

pub(super) fn clean_widened_f32(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for child in map.values_mut() {
                clean_widened_f32(child);
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                clean_widened_f32(child);
            }
        }
        Value::Number(n) => {
            // 只动真正的浮点 Number；整数（端口/条数等）不得被改写成 `8000.0`。
            // `g.to_string()` 是 f32 的最短 round-trip 十进制（Rust Display 保证），
            // 重新解析成 f64 建 Number：既收短了小数位，又保住 "1.0" 的浮点形态
            //（直接建整数 Number 会把 `speed: 1.0` 写成 `1`）。
            if n.is_f64() {
                let f = n.as_f64().unwrap_or_default();
                let g = f as f32;
                if (g as f64) == f {
                    if let Ok(x) = g.to_string().parse::<f64>() {
                        if let Some(num) = serde_json::Number::from_f64(x) {
                            *v = Value::Number(num);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}
