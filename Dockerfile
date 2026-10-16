# syntax=docker/dockerfile:1
# WCM Log Forwarder — 測試用 image
# 兩階段建置：rust 編譯 → debian slim 執行環境（僅含 forwarder binary）。
# 設定檔與 plugin 於 docker run 時掛載：
#   -v <forwarder.yaml>:/app/forwarder.yaml:ro
#   -v <plugin 目錄>:/app/plugin:ro
# （plugin 路徑由 forwarder 以「設定檔所在目錄/plugin/」解析，見 src/main.rs）

FROM rust:1.93-slim-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY wit ./wit
# 可選：--build-arg CARGO_BUILD_JOBS=4 限制編譯平行度（未給時與原本相同，用全部核心）
ARG CARGO_BUILD_JOBS=
# cache mount：cargo registry 與 target 跨次建置重用，改 src 後重建只需增量編譯
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release ${CARGO_BUILD_JOBS:+-j $CARGO_BUILD_JOBS} \
    && cp target/release/wcm-base-log-forwarder /build/wcm-base-log-forwarder

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/wcm-base-log-forwarder /usr/local/bin/wcm-base-log-forwarder
WORKDIR /app
ENTRYPOINT ["/usr/local/bin/wcm-base-log-forwarder"]
CMD ["/app/forwarder.yaml"]
