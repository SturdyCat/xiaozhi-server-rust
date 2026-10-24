//! ONNX `TensorProto.raw_data` → typed 字段重写（上游 `controller/tools/silero_typed.py`
//! 的 Rust 等价实现）。
//!
//! ## 为什么必须重写
//!
//! 训练/导出工具默认把权重塞进 `TensorProto.raw_data`——一段**没有任何对齐保证**的裸
//! 字节。armv7 上的 onnxruntime 1.19 在 `CreateSession` 里会把它当 `float*`/`int64_t*`
//! 直接解引用，遇到未对齐地址就是 SIGBUS（整个进程被杀，没有错误码）。把同一批数据
//! 展开成 `float_data`/`int64_data` 这类 typed 字段后，ORT 走按元素解码，不再踩裸指针。
//!
//! ## 为什么是"自己实现 protobuf"
//!
//! 只为一个字段搬迁引入 `prost` + `onnx-proto` 两个重量级依赖不划算；这里的遍历面很窄
//! （ModelProto → GraphProto → NodeProto/AttributeProto → TensorProto），按需解析即可，
//! 其余字段**原样搬运**，不做重组，避免改动未知字段。
//!
//! ## 字节级等价
//!
//! 与 Python 版保持一致的两个细节（都在 `onnx.proto` 里定死，实测核对过）：
//!
//! - `dims` 是**非 packed**（`repeated int64 dims = 1;` 没写 `[packed = true]`），
//!   原样搬运即可；
//! - `float_data` / `int64_data` / `int32_data` / `double_data` / `uint64_data` 是
//!   **packed**（tag 的线型为 2，`<tag><长度><载荷>`）。
//!
//! 同时把被修改的 `TensorProto` 按字段号稳定排序后再输出——protobuf 序列化本来就按
//! 字段号升序，这样重写结果与 Python 版**逐比特相同**（可用 `ONNX_TYPED_FIXTURE`
//! 那条 `#[ignore]` 测试实测核对）。
//!
//! 幂等：已经是 typed 的输入原样返回，`changed == false`。

use std::borrow::Cow;

/// 重写结果。`changed` 为真表示确实搬过 `raw_data`，否则 `bytes` 等于输入。
#[derive(Debug)]
pub(super) struct Rewritten {
    pub bytes: Vec<u8>,
    pub changed: bool,
}

// ---- protobuf 字段号（onnx.proto：ModelProto / GraphProto / NodeProto / AttributeProto）----
const MODEL_GRAPH: u32 = 7;
const GRAPH_NODE: u32 = 1;
const GRAPH_INITIALIZER: u32 = 5;
const NODE_ATTRIBUTE: u32 = 5;
const ATTR_TENSOR: u32 = 5;
const ATTR_GRAPH: u32 = 6;
const ATTR_GRAPHS: u32 = 10;

// ---- TensorProto ----
const TENSOR_DATA_TYPE: u32 = 2;
const TENSOR_FLOAT_DATA: u32 = 4;
const TENSOR_INT32_DATA: u32 = 5;
const TENSOR_INT64_DATA: u32 = 7;
const TENSOR_RAW_DATA: u32 = 9;
const TENSOR_DOUBLE_DATA: u32 = 10;
const TENSOR_UINT64_DATA: u32 = 11;

// ---- protobuf 线型 ----
const WIRE_VARINT: u8 = 0;
const WIRE_FIXED64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_FIXED32: u8 = 5;

// ---- TensorProto.DataType ----
const DT_FLOAT: i32 = 1;
const DT_UINT8: i32 = 2;
const DT_INT8: i32 = 3;
const DT_UINT16: i32 = 4;
const DT_INT16: i32 = 5;
const DT_INT32: i32 = 6;
const DT_INT64: i32 = 7;
const DT_BOOL: i32 = 9;
const DT_FLOAT16: i32 = 10;
const DT_DOUBLE: i32 = 11;
const DT_UINT32: i32 = 12;
const DT_UINT64: i32 = 13;
const DT_BFLOAT16: i32 = 16;

type Visit<'a> = &'a mut dyn FnMut(u32, &[u8]) -> Result<Option<Vec<u8>>, String>;

/// 把模型里所有 `raw_data` 展开成 typed 字段。
pub(super) fn rewrite(model: &[u8]) -> Result<Rewritten, String> {
    let (bytes, changed) = rewrite_model(model)?;
    Ok(Rewritten { bytes, changed })
}

fn rewrite_model(buf: &[u8]) -> Result<(Vec<u8>, bool), String> {
    walk(buf, &mut |number, body| match number {
        MODEL_GRAPH => Ok(Some(rewrite_graph(body)?)),
        _ => Ok(None),
    })
}

/// 接收一个 message 的**长度定界**子字段，返回替换后的载荷（`None` = 原样保留）。
///
/// 只有线型 2 的字段会被回调；varint/fixed 字段原样搬运——模型里需要改的字段全部是
/// 长度定界的嵌套 message。
fn walk(buf: &[u8], visit: Visit<'_>) -> Result<(Vec<u8>, bool), String> {
    let mut out = Vec::with_capacity(buf.len() + 64);
    let mut changed = false;
    let mut pos = 0usize;
    while pos < buf.len() {
        let start = pos;
        let (key, used) = read_varint(&buf[pos..]).ok_or("字段键被截断")?;
        pos += used;
        let number = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        if number == 0 {
            return Err("字段号为 0（非法）".to_string());
        }
        match wire {
            WIRE_VARINT => {
                let (_, used) = read_varint(&buf[pos..])
                    .ok_or_else(|| format!("字段 {number} 的 varint 值被截断"))?;
                pos += used;
                out.extend_from_slice(&buf[start..pos]);
            }
            WIRE_FIXED64 => {
                pos = advance(pos, 8, buf.len(), number)?;
                out.extend_from_slice(&buf[start..pos]);
            }
            WIRE_FIXED32 => {
                pos = advance(pos, 4, buf.len(), number)?;
                out.extend_from_slice(&buf[start..pos]);
            }
            WIRE_LEN => {
                let (len, used) = read_varint(&buf[pos..])
                    .ok_or_else(|| format!("字段 {number} 的长度被截断"))?;
                let body_start = pos + used;
                let body_end = body_start
                    .checked_add(len as usize)
                    .filter(|end| *end <= buf.len())
                    .ok_or_else(|| format!("字段 {number} 的长度越界"))?;
                let body = &buf[body_start..body_end];
                match visit(number, body)? {
                    Some(replacement) => {
                        changed |= replacement.as_slice() != body;
                        write_key(&mut out, number, WIRE_LEN);
                        write_varint(&mut out, replacement.len() as u64);
                        out.extend_from_slice(&replacement);
                    }
                    None => out.extend_from_slice(&buf[start..body_end]),
                }
                pos = body_end;
            }
            other => return Err(format!("不支持的 protobuf 线型 {other}（字段 {number}）")),
        }
    }
    Ok((out, changed))
}

fn rewrite_graph(buf: &[u8]) -> Result<Vec<u8>, String> {
    let (out, _) = walk(buf, &mut |number, body| match number {
        GRAPH_NODE => Ok(Some(rewrite_node(body)?)),
        GRAPH_INITIALIZER => Ok(Some(rewrite_tensor(body)?)),
        _ => Ok(None),
    })?;
    Ok(out)
}

fn rewrite_node(buf: &[u8]) -> Result<Vec<u8>, String> {
    let (out, _) = walk(buf, &mut |number, body| match number {
        NODE_ATTRIBUTE => Ok(Some(rewrite_attribute(body)?)),
        _ => Ok(None),
    })?;
    Ok(out)
}

fn rewrite_attribute(buf: &[u8]) -> Result<Vec<u8>, String> {
    let (out, _) = walk(buf, &mut |number, body| match number {
        ATTR_TENSOR => Ok(Some(rewrite_tensor(body)?)),
        ATTR_GRAPH | ATTR_GRAPHS => Ok(Some(rewrite_graph(body)?)),
        _ => Ok(None),
    })?;
    Ok(out)
}

/// 单个 `TensorProto`：删 `raw_data`、按 `data_type` 生成 typed 字段，再按字段号排序。
fn rewrite_tensor(buf: &[u8]) -> Result<Vec<u8>, String> {
    let mut segments: Vec<(u32, Cow<'_, [u8]>)> = Vec::new();
    let mut data_type: Option<i32> = None;
    let mut raw: Option<&[u8]> = None;
    let mut pos = 0usize;
    while pos < buf.len() {
        let start = pos;
        let (key, used) = read_varint(&buf[pos..]).ok_or("TensorProto 字段键被截断")?;
        pos += used;
        let number = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        match wire {
            WIRE_VARINT => {
                let (value, used) = read_varint(&buf[pos..])
                    .ok_or_else(|| format!("TensorProto 字段 {number} 被截断"))?;
                pos += used;
                if number == TENSOR_DATA_TYPE {
                    data_type = Some(value as i32);
                }
            }
            WIRE_FIXED64 => pos = advance(pos, 8, buf.len(), number)?,
            WIRE_FIXED32 => pos = advance(pos, 4, buf.len(), number)?,
            WIRE_LEN => {
                let (len, used) = read_varint(&buf[pos..])
                    .ok_or_else(|| format!("TensorProto 字段 {number} 的长度被截断"))?;
                let body_start = pos + used;
                let body_end = body_start
                    .checked_add(len as usize)
                    .filter(|end| *end <= buf.len())
                    .ok_or_else(|| format!("TensorProto 字段 {number} 的长度越界"))?;
                if number == TENSOR_RAW_DATA {
                    raw = Some(&buf[body_start..body_end]);
                }
                pos = body_end;
            }
            other => {
                return Err(format!(
                    "TensorProto 出现不支持的线型 {other}（字段 {number}）"
                ))
            }
        }
        if number != TENSOR_RAW_DATA {
            segments.push((number, Cow::Borrowed(&buf[start..pos])));
        }
    }

    // 没有 raw_data（或空载荷）→ 已是 typed 形态，一个字节都不动。
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        return Ok(buf.to_vec());
    };
    let data_type = data_type.ok_or("TensorProto 缺少 data_type")?;
    let (field, encoded) = typed_field(data_type, raw)?;
    segments.push((field, Cow::Owned(encoded)));
    // 稳定排序 = 同字段多值保序；字段号升序与 protobuf 序列化一致（逐比特等价的关键）。
    segments.sort_by_key(|(number, _)| *number);

    let mut out = Vec::with_capacity(buf.len() + 64);
    for (_, bytes) in segments {
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

/// `raw_data` → `(typed 字段号, 完整编码)`。
fn typed_field(data_type: i32, raw: &[u8]) -> Result<(u32, Vec<u8>), String> {
    match data_type {
        // 小端定长数据可以直接当 packed 载荷用（float32/f64 与内存布局一致）。
        DT_FLOAT => Ok((TENSOR_FLOAT_DATA, packed(TENSOR_FLOAT_DATA, raw))),
        DT_DOUBLE => Ok((TENSOR_DOUBLE_DATA, packed(TENSOR_DOUBLE_DATA, raw))),
        DT_INT64 => {
            let mut payload = Vec::with_capacity(raw.len());
            for chunk in chunks(raw, 8, "INT64")? {
                write_varint(&mut payload, i64::from_le_bytes(as_array(chunk)) as u64);
            }
            Ok((TENSOR_INT64_DATA, packed(TENSOR_INT64_DATA, &payload)))
        }
        DT_UINT32 | DT_UINT64 => {
            let width = if data_type == DT_UINT32 { 4 } else { 8 };
            let mut payload = Vec::with_capacity(raw.len());
            for chunk in chunks(raw, width, "UINT")? {
                let value = if width == 4 {
                    u64::from(u32::from_le_bytes(as_array(chunk)))
                } else {
                    u64::from_le_bytes(as_array(chunk))
                };
                write_varint(&mut payload, value);
            }
            Ok((TENSOR_UINT64_DATA, packed(TENSOR_UINT64_DATA, &payload)))
        }
        // 其余整数/布尔/半精度：ONNX 统一落到 int32_data。
        other => {
            let width = match other {
                DT_UINT8 | DT_INT8 | DT_BOOL => 1,
                DT_UINT16 | DT_INT16 | DT_FLOAT16 | DT_BFLOAT16 => 2,
                DT_INT32 => 4,
                _ => return Err(format!("不支持的 TensorProto.data_type={other}")),
            };
            let mut payload = Vec::with_capacity(raw.len() * 2);
            for chunk in chunks(raw, width, "整数")? {
                let value: i64 = match other {
                    DT_UINT8 | DT_BOOL => i64::from(chunk[0]),
                    DT_INT8 => i64::from(i8::from_le_bytes(as_array(chunk))),
                    DT_UINT16 | DT_FLOAT16 | DT_BFLOAT16 => {
                        i64::from(u16::from_le_bytes(as_array(chunk)))
                    }
                    DT_INT16 => i64::from(i16::from_le_bytes(as_array(chunk))),
                    DT_INT32 => i64::from(i32::from_le_bytes(as_array(chunk))),
                    _ => unreachable!("width 已覆盖全部分支"),
                };
                // 负数按 64 位补码编码（protobuf 对 int32/int64 的负数都是 10 字节）。
                write_varint(&mut payload, value as u64);
            }
            Ok((TENSOR_INT32_DATA, packed(TENSOR_INT32_DATA, &payload)))
        }
    }
}

fn chunks<'a>(
    raw: &'a [u8],
    width: usize,
    what: &str,
) -> Result<std::slice::ChunksExact<'a, u8>, String> {
    if raw.len() % width != 0 {
        return Err(format!(
            "{what} raw_data 长度 {} 不是 {width} 的整数倍",
            raw.len()
        ));
    }
    Ok(raw.chunks_exact(width))
}

fn as_array<const N: usize>(chunk: &[u8]) -> [u8; N] {
    chunk.try_into().expect("宽度已由 chunks 校验")
}

fn packed(field: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    write_key(&mut out, field, WIRE_LEN);
    write_varint(&mut out, payload.len() as u64);
    out.extend_from_slice(payload);
    out
}

fn advance(pos: usize, width: usize, limit: usize, number: u32) -> Result<usize, String> {
    pos.checked_add(width)
        .filter(|end| *end <= limit)
        .ok_or_else(|| format!("字段 {number} 的定长值被截断"))
}

fn read_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (index, byte) in buf.iter().take(10).enumerate() {
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
        shift += 7;
    }
    None
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn write_key(out: &mut Vec<u8>, number: u32, wire: u8) {
    write_varint(out, (u64::from(number) << 3) | u64::from(wire));
}

#[cfg(test)]
#[path = "onnx_typed_tests.rs"]
mod tests;
