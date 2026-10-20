//! main.rs 默认日志过滤（[`super::default_log_filter`]）的回归测试。
//!
//! 背景（2026-10 事故）：镜像与 compose 曾内置 `RUST_LOG=info`，使 main.rs 的兜底
//! 过滤永不生效，`ort::logging` 的会话初始化明细（每次加载模型上百行 INFO）刷满
//! docker logs。现部署文件不再内置 RUST_LOG，由兜底保证：明细压到 WARN（默认不
//! 显示、`RUST_LOG=debug` 可见），ort 本体 dylib 加载提示保留 INFO。

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use super::default_log_filter;

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
fn debug_filter_shows_ort_logging_details() {
    // 排障语义：显式给 debug 档时明细全量可见（即"明细等效 DEBUG 级"）。
    let out = capture("debug", || {
        tracing::info!(target: "ort::logging", "Session Options {{ intra_op_param: .. }}");
    });
    assert!(
        out.contains("Session Options"),
        "debug 档下明细应可见：{out}"
    );
}
