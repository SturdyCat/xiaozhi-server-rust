# 小智插件构建镜像（默认插件启动引导用；docker build -t xiaozhi-plugin-builder .）
# 产物策略（用户方向 2026-11-04）：服务端零本地构建——缺产物即调本镜像构建。
FROM rust:1-slim

RUN rustup target add wasm32-unknown-unknown

# wasm-tools：组件化打包（guest 是 core wasm + component new）
RUN cargo install --locked wasm-tools && rm -rf ${CARGO_HOME:-/usr/local/cargo}/registry

# 构建脚本在插件目录内（build.sh 写产物到 server/tests/fixtures/wasm/，
# 由宿主 bootstrap 读取后采用到数据卷 plugins/）
WORKDIR /work
ENTRYPOINT ["bash"]
