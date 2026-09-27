# 多阶段构建：默认特性仅含 mock，可使用 `--features sherpa` 启用真实引擎。
# 真实引擎需要 libopus（audiopus）与网络连接（构建脚本自动下载 sherpa-onnx 原生库）。

# ---------- 构建阶段 ----------
FROM rust:1.82-bookworm AS builder
WORKDIR /app

# 系统依赖：audiopus 编译/链接需要 pkg-config 与 libopus
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libopus-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY src ./src

# 在 N5105（Jasper Lake）上构建时启用本地指令集优化
ENV RUSTFLAGS="-C target-cpu=native"
RUN cargo build --release --features sherpa

# ---------- 运行阶段 ----------
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# 运行期需要 libopus 动态库（audiopus 链接）
RUN apt-get update \
    && apt-get install -y --no-install-recommends libopus0 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/xiaozhi-server-rust /app/server

EXPOSE 8000
ENTRYPOINT ["/app/server"]
