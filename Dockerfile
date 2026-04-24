# syntax=docker/dockerfile:1.7

FROM rust:1.88-bookworm AS builder

WORKDIR /workspace

RUN set -eux; \
    apt-get update -o Acquire::Retries=5; \
    apt-get install -y --no-install-recommends \
      libasound2-dev \
      libwebkit2gtk-4.1-dev \
      libappindicator3-dev \
      librsvg2-dev \
      patchelf \
      pkg-config; \
    rm -rf /var/lib/apt/lists/*

COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
COPY src-tauri/build.rs ./src-tauri/build.rs
COPY src-tauri/patches ./src-tauri/patches
COPY src-tauri/migrations ./src-tauri/migrations
COPY src-tauri/src ./src-tauri/src
COPY src-tauri/capabilities ./src-tauri/capabilities
COPY src-tauri/icons ./src-tauri/icons
COPY src-tauri/tauri.conf.json ./src-tauri/tauri.conf.json

RUN cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin fini

FROM ubuntu:24.04 AS runtime

RUN set -eux; \
    apt-get update -o Acquire::Retries=5; \
    apt-get install -y --no-install-recommends --fix-missing \
      ca-certificates \
      libasound2t64 \
      libgtk-3-0 \
      libwebkit2gtk-4.1-0 \
      librsvg2-2; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/src-tauri/target/release/fini /usr/local/bin/fini

ENV XDG_DATA_HOME=/data

VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/fini"]
CMD ["mcp"]

# ── E2E test stage ─────────────────────────────────────────────────────────────
FROM node:24-slim AS test

RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-3-0 \
    libwebkit2gtk-4.1-0 \
    libayatana-appindicator3-1 \
    librsvg2-2 \
    libglib2.0-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/src-tauri/target/release/fini /usr/local/bin/fini

WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm install

COPY tsconfig*.json ./
COPY specs/e2e ./specs/e2e

ENV FINI_BINARY=/usr/local/bin/fini \
    TZ=UTC

CMD ["npm", "run", "test:e2e"]
