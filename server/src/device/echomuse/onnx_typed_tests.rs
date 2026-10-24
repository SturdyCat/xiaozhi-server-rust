//! `onnx_typed` 契约：`raw_data` 必须搬成 packed typed 字段、字段号升序、幂等，
//! 并对畸形输入给出可定位的错误（而不是产出半个模型发给设备）。
use super::*;

// ---- 最小 protobuf 构造器（只覆盖 ModelProto/GraphProto/TensorProto 用到的线型） ----

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn varint_field(number: u32, value: u64) -> Vec<u8> {
    let mut out = varint((u64::from(number) << 3) | u64::from(WIRE_VARINT));
    out.extend(varint(value));
    out
}

fn bytes_field(number: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = varint((u64::from(number) << 3) | u64::from(WIRE_LEN));
    out.extend(varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

/// 按 `dims(1) → data_type(2) → name(8) → raw_data(9)` 的升序构造（与 protobuf 序列化同形）。
fn tensor(data_type: i32, dims: &[i64], name: &str, raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for dim in dims {
        out.extend(varint_field(1, *dim as u64));
    }
    out.extend(varint_field(2, data_type as u64));
    out.extend(bytes_field(8, name.as_bytes()));
    if !raw.is_empty() {
        out.extend(bytes_field(9, raw));
    }
    out
}

fn model_of(initializer: &[u8]) -> Vec<u8> {
    let mut graph = bytes_field(GRAPH_INITIALIZER, initializer);
    graph.extend(bytes_field(2, b"g"));
    bytes_field(MODEL_GRAPH, &graph)
}

/// 逐字段拆开一个 message（够用即可：本项目只关心 varint / fixed / 长度定界）。
fn fields(buf: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let (key, used) = read_varint(&buf[pos..]).expect("key");
        pos += used;
        let number = (key >> 3) as u32;
        match (key & 0x07) as u8 {
            WIRE_VARINT => {
                let (value, used) = read_varint(&buf[pos..]).expect("varint");
                pos += used;
                out.push((number, varint(value)));
            }
            WIRE_FIXED64 => {
                out.push((number, buf[pos..pos + 8].to_vec()));
                pos += 8;
            }
            WIRE_FIXED32 => {
                out.push((number, buf[pos..pos + 4].to_vec()));
                pos += 4;
            }
            WIRE_LEN => {
                let (len, used) = read_varint(&buf[pos..]).expect("len");
                pos += used;
                out.push((number, buf[pos..pos + len as usize].to_vec()));
                pos += len as usize;
            }
            other => panic!("unexpected wire type {other}"),
        }
    }
    out
}

/// `model → graph → initializer` 三层取到被重写的那个 TensorProto。
fn initializer_of(model: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let graph = fields(model)
        .into_iter()
        .find(|(number, _)| *number == MODEL_GRAPH)
        .expect("graph")
        .1;
    let tensor = fields(&graph)
        .into_iter()
        .find(|(number, _)| *number == GRAPH_INITIALIZER)
        .expect("initializer")
        .1;
    fields(&tensor)
}

#[test]
fn float_raw_data_becomes_packed_float_data() {
    // 1.0f32 / 2.0f32 的小端字节。
    let raw = [0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x40];
    let rewritten = rewrite(&model_of(&tensor(DT_FLOAT, &[2], "w", &raw))).expect("rewrite");
    assert!(rewritten.changed, "raw_data 必须被搬走");

    let tensor = initializer_of(&rewritten.bytes);
    assert!(
        tensor.iter().all(|(number, _)| *number != TENSOR_RAW_DATA),
        "raw_data 必须消失（否则 armv7 ORT 仍会 SIGBUS）"
    );
    let float_data: Vec<Vec<u8>> = tensor
        .iter()
        .filter(|(number, _)| *number == TENSOR_FLOAT_DATA)
        .map(|(_, body)| body.clone())
        .collect();
    assert_eq!(
        float_data,
        vec![raw.to_vec()],
        "float_data 载荷应逐字节等值"
    );
    // dims 是非 packed，原样保留两条。
    let dims: Vec<_> = tensor.iter().filter(|(number, _)| *number == 1).collect();
    assert_eq!(dims.len(), 1);
    assert_eq!(dims[0].1, varint(2));
    // 字段号升序：1 dims → 2 data_type → 4 float_data → 8 name。
    let order: Vec<u32> = tensor.iter().map(|(number, _)| *number).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "字段必须按号升序输出");
}

#[test]
fn rewrite_is_idempotent() {
    let once = rewrite(&model_of(&tensor(DT_FLOAT, &[1], "w", &[0, 0, 0, 0]))).expect("rewrite");
    let twice = rewrite(&once.bytes).expect("rewrite again");
    assert!(!twice.changed, "typed 形态不该再被判为需要重写");
    assert_eq!(twice.bytes, once.bytes);
}

#[test]
fn int64_raw_data_uses_packed_varints() {
    // 7 与 -1：后者按 64 位补码编码成 10 字节。
    let mut raw = 7i64.to_le_bytes().to_vec();
    raw.extend((-1i64).to_le_bytes());
    let rewritten = rewrite(&model_of(&tensor(DT_INT64, &[2], "s", &raw))).expect("rewrite");

    let body = initializer_of(&rewritten.bytes)
        .into_iter()
        .find(|(number, _)| *number == TENSOR_INT64_DATA)
        .expect("int64_data")
        .1;
    let mut expected = varint(7);
    expected.extend([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
    assert_eq!(body, expected);
}

#[test]
fn int8_negative_values_use_ten_byte_varints() {
    // 只有 int32_data 一条路：int8 的 -2 也要按 64 位补码写（protobuf 对 int32 负数如此）。
    let rewritten = rewrite(&model_of(&tensor(DT_INT8, &[1], "q", &[0xFE]))).expect("rewrite");
    let body = initializer_of(&rewritten.bytes)
        .into_iter()
        .find(|(number, _)| *number == TENSOR_INT32_DATA)
        .expect("int32_data")
        .1;
    assert_eq!(body.len(), 10, "负数必须是 10 字节补码");
    assert_eq!(body[0], 0xFE);
    assert_eq!(*body.last().unwrap(), 0x01);
}

#[test]
fn empty_raw_data_is_left_alone() {
    let model = model_of(&tensor(DT_FLOAT, &[1], "w", &[]));
    let rewritten = rewrite(&model).expect("rewrite");
    assert!(!rewritten.changed);
    assert_eq!(rewritten.bytes, model);
}

#[test]
fn truncated_model_is_rejected() {
    let model = model_of(&tensor(DT_FLOAT, &[1], "w", &[0, 0, 0, 0]));
    let error = rewrite(&model[..model.len() - 2]).expect_err("截断必须报错");
    assert!(!error.is_empty());
}

#[test]
fn unsupported_data_type_names_the_type() {
    // 8 = STRING：raw_data 对它没有 typed 字段可落，宁可报错也不发半个模型。
    let error = rewrite(&model_of(&tensor(8, &[1], "w", &[0x41]))).expect_err("string 不支持");
    assert!(error.contains("data_type=8"), "error was: {error}");
}

/// 真模型核对：拿上游原始 `silero_vad.onnx` 跑一遍，确认重写后的 sha256 与
/// Python `silero_typed.py` 的产物一致（逐比特相同）。
///
/// ```text
/// ONNX_TYPED_FIXTURE=/tmp/silero_vad_raw.onnx \
/// ONNX_TYPED_EXPECT_SHA256=da5e8a9c… \
///   cargo test --manifest-path server/Cargo.toml fixture_rewrite -- --ignored --nocapture
/// ```
#[test]
#[ignore = "需要真实模型：ONNX_TYPED_FIXTURE=<原始 onnx> [ONNX_TYPED_EXPECT_SHA256=<期望>]"]
fn fixture_rewrite_matches_the_python_typed_rewrite() {
    let path = std::env::var("ONNX_TYPED_FIXTURE").expect("ONNX_TYPED_FIXTURE");
    let raw = std::fs::read(&path).expect("读不到 fixture");
    let rewritten = rewrite(&raw).expect("rewrite");
    let digest = crate::device::echomuse::shell::sha256_hex(&rewritten.bytes);
    println!(
        "raw={} bytes, typed={} bytes, changed={}, sha256={digest}",
        raw.len(),
        rewritten.bytes.len(),
        rewritten.changed
    );
    if let Ok(expected) = std::env::var("ONNX_TYPED_EXPECT_SHA256") {
        assert_eq!(digest, expected.trim().to_lowercase());
    }
}
