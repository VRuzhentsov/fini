# syntax=docker/dockerfile:1.7

FROM node:24.15.0-trixie-slim AS node-deps

WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm ci

FROM node-deps AS playwright-browsers

RUN npx playwright install chromium

FROM rust:1.88-bookworm AS rust-builder-base

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

FROM rust-builder-base AS app-build-release

RUN cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin fini

FROM rust-builder-base AS app-build-e2e

COPY --from=node-deps /usr/local/ /usr/local/

WORKDIR /workspace

COPY package.json package-lock.json* ./
COPY --from=node-deps /app/node_modules ./node_modules
COPY tsconfig*.json ./
COPY src ./src
COPY vite.config.ts ./
COPY index.html ./

RUN node ./node_modules/@tauri-apps/cli/tauri.js build --debug --features e2e-testing --no-bundle

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

COPY --from=app-build-release /workspace/src-tauri/target/release/fini /usr/local/bin/fini

ENV XDG_DATA_HOME=/data

VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/fini"]

# ── E2E test stage ─────────────────────────────────────────────────────────────
FROM node:24.15.0-trixie-slim AS test

# Fini binary runtime libs + Playwright Chromium system deps (fonts, nss/nspr,
# dbus, X11 bits). The browser binary comes from the cached Playwright stage;
# this apt layer provides the runtime libraries it expects.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-3-0 \
    libwebkit2gtk-4.1-0 \
    libayatana-appindicator3-1 \
    librsvg2-2 \
    libglib2.0-0 \
    libnss3 \
    libnspr4 \
    libdbus-1-3 \
    libatk1.0-0 \
    libatk-bridge2.0-0 \
    libcups2 \
    libx11-6 \
    libxcomposite1 \
    libxdamage1 \
    libxext6 \
    libxfixes3 \
    libxrandr2 \
    libxkbcommon0 \
    libpango-1.0-0 \
    libcairo2 \
    libasound2 \
    fonts-liberation \
    ca-certificates \
    xvfb \
    && rm -rf /var/lib/apt/lists/*

COPY --from=app-build-e2e /workspace/src-tauri/target/debug/fini /usr/local/bin/fini

WORKDIR /app

COPY package.json package-lock.json* ./
COPY --from=node-deps /app/node_modules ./node_modules
COPY --from=playwright-browsers /root/.cache/ms-playwright /root/.cache/ms-playwright

COPY tsconfig*.json ./
COPY src ./src
COPY vite.config.ts ./
COPY index.html ./
COPY specs/e2e ./specs/e2e

ENV FINI_BINARY=/usr/local/bin/fini \
    TZ=UTC \
    CI=1

CMD ["npm", "run", "test:e2e:ci"]
