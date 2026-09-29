# 多阶段构建：默认特性仅含 mock，可使用 `--features sherpa` 启用真实引擎。
# 真实引擎需要 libopus（audiopus）与网络连接（构建脚本自动下载 sherpa-onnx 原生库）。

# ---------- 构建阶段 ----------
FROM rust:1.90-bookworm AS builder
WORKDIR /app

# 系统依赖：audiopus 编译/链接需要 pkg-config 与 libopus
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libopus-dev \
    && rm -rf /var/lib/apt/lists/*

# 源码与 Cargo 清单位于 monorepo 的 server/ 子目录
COPY server/Cargo.toml server/Cargo.lock* ./
COPY server/src ./src

# 目标指令集固定为 x86-64-v2：部署机 N5105（Tremont，无 AVX）可安全运行。
# 不要用 native —— CI 构建机的 CPU 与部署机 N5105 不同，native 会嵌入部署机不支持的
# AVX/AVX2/AVX-512 指令，导致运行时 SIGILL 崩溃。
ENV RUSTFLAGS="-C target-cpu=x86-64-v2"
RUN cargo build --release --features sherpa

# ---------- 运行阶段 ----------
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# 运行期需要 libopus 动态库（audiopus 链接）；
# curl/bzip2 供 entrypoint 在模型缺失时按需下载（见 docker-entrypoint.sh）。
RUN apt-get update \
    && apt-get install -y --no-install-recommends libopus0 ca-certificates curl bzip2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/xiaozhi-server-rust /app/server
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

EXPOSE 8000
ENTRYPOINT ["/app/docker-entrypoint.sh"]
