//! main.rs 日志过滤（[`super::resolved_log_filter`] / [`super::default_log_filter`]）的回归测试。
//!
//! 背景（2026-10 事故）：镜像与 compose 曾内置 `RUST_LOG=info`，使当时的兜底过滤
//! 永不生效，`ort::logging` 的会话初始化明细（每次加载模型上百行 INFO）刷满
//! docker logs。现约定两级保证：部署文件一律不内置 RUST_LOG 默认值；即便 RUST_LOG
//! 存在（部署模板残留、手工 `-e`），只要没显式写 `ort` 目标指令，也自动追加
//! `ort::logging=warn` 降噪底座——明细默认不可见，ort 本体 dylib 加载提示保留
//! INFO。要 ort 全量明细：`RUST_LOG=debug,ort::logging=debug`。

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use super::{default_log_filter, resolved_log_filter};

/// 共享内存缓冲：捕获 fmt 层输出（`MakeWriter` 要求返回值活到 'a，用 Arc 共享克隆）。
#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("日志缓冲锁").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buf {
    type Writer = Buf;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// 用给定过滤表达式构造 fmt 订阅者，在同线程跑一段事件产出，返回捕获的控制台文本。
/// （`with_default` 是线程局部的，测试间互不污染。）
fn capture(filter: &str, emit: impl FnOnce()) -> String {
    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_writer(buf.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, emit);
    // 锁卫不能留在尾表达式里：块尾临时晚于局部 buf 析构（新版 rustc 报 E0597），
    // 先克隆到局部再转 String。
    let bytes = buf.0.lock().expect("日志缓冲锁").clone();
    String::from_utf8(bytes).expect("日志输出是 UTF-8")
}

#[test]
fn default_filter_keeps_ort_dylib_info_and_app_logs_hides_ort_logging_details() {
    let out = capture(&default_log_filter(), || {
        tracing::info!(target: "ort", "Loaded ONNX Runtime dylib with version '1.22.1'");
        tracing::info!(target: "ort::logging", "Session Options {{ intra_op_param: .. }}");
        tracing::info!(target: "ort::logging", "GraphTransformer BiasGeluFusion modified: 1");
        tracing::warn!(target: "ort::logging", "Some nodes were still assigned to CPU.");
        tracing::info!(target: "xiaozhi_server_rust", "业务日志：配置加载完成");
    });
    assert!(
        out.contains("Loaded ONNX Runtime dylib"),
        "ort 本体 info 应保留：{out}"
    );
    assert!(
        out.contains("业务日志：配置加载完成"),
        "业务 info 应保留：{out}"
    );
    assert!(
        out.contains("assigned to CPU"),
        "ort::logging 的 WARN 应保留：{out}"
    );
    assert!(
        !out.contains("Session Options"),
        "ort::logging 的 INFO 明细应压掉：{out}"
    );
    assert!(
        !out.contains("GraphTransformer"),
        "ort::logging 的 INFO 明细应压掉：{out}"
    );
}

#[test]
fn rust_log_set_without_ort_directive_still_suppresses_details() {
    // 本轮修复的回归：部署模板残留 `RUST_LOG=info` 时，ort 明细仍被降噪底座压掉，
    // 不再依赖"RUST_LOG 未设置"才生效（2026-10 事故的复发路径）。
    let expr = resolved_log_filter(Some("info"));
    assert_eq!(expr, "info,ort::logging=warn");
    let out = capture(&expr, || {
        tracing::info!(target: "ort", "Loaded ONNX Runtime dylib with version '1.22.1'");
        tracing::info!(target: "ort::logging", "Session Options {{ intra_op_param: .. }}");
        tracing::warn!(target: "ort::logging", "Some nodes were still assigned to CPU.");
        tracing::info!(target: "xiaozhi_server_rust", "业务日志：配置加载完成");
    });
    assert!(
        out.contains("Loaded ONNX Runtime dylib"),
        "ort 本体 info 应保留：{out}"
    );
    assert!(
        out.contains("业务日志：配置加载完成"),
        "业务 info 应保留：{out}"
    );
    assert!(
        out.contains("assigned to CPU"),
        "ort::logging 的 WARN 应保留：{out}"
    );
    assert!(
        !out.contains("Session Options"),
        "RUST_LOG=info 时 ort::logging 的 INFO 明细应压掉：{out}"
    );
}

#[test]
fn rust_log_debug_gets_noise_floor_appended() {
    // debug 档：业务明细全量可见，ort 明细仍压 warn；要 ort 明细必须显式写 ort 目标。
    let expr = resolved_log_filter(Some("debug"));
    assert_eq!(expr, "debug,ort::logging=warn");
    let out = capture(&expr, || {
        tracing::debug!(target: "xiaozhi_server_rust", "业务明细：分段延迟");
        tracing::info!(target: "ort::logging", "Session Options {{ intra_op_param: .. }}");
    });
    assert!(
        out.contains("业务明细：分段延迟"),
        "debug 档业务明细可见：{out}"
    );
    assert!(
        !out.contains("Session Options"),
        "debug 档 ort 明细仍应压掉，显式 ort 指令才可见：{out}"
    );
}

#[test]
fn rust_log_with_explicit_ort_directive_is_respected() {
    // 用户显式写 ort 目标指令（含 `ort=…` 与 span 过滤写法）时完全尊重，不追加底座。
    for spec in ["info,ort::logging=info", "ort=debug", "ort[load]=info"] {
        assert_eq!(
            resolved_log_filter(Some(spec)),
            spec,
            "显式 ort 指令应原样保留"
        );
    }
    let out = capture("info,ort::logging=info", || {
        tracing::info!(target: "ort::logging", "Session Options {{ intra_op_param: .. }}");
    });
    assert!(
        out.contains("Session Options"),
        "显式 ort::logging=info 时明细应可见：{out}"
    );
}

#[test]
fn rust_log_blank_or_invalid_falls_back_to_baseline() {
    assert_eq!(resolved_log_filter(None), default_log_filter());
    assert_eq!(resolved_log_filter(Some("")), default_log_filter());
    assert_eq!(resolved_log_filter(Some("  ")), default_log_filter());
    assert_eq!(resolved_log_filter(Some(",")), default_log_filter());
    // 非法级别原样透传，由 build_log_filter 解析失败后回退基线（同降噪）。
    assert_eq!(
        resolved_log_filter(Some("not-a-level")),
        "not-a-level,ort::logging=warn"
    );
}
