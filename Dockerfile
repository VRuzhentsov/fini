# syntax=docker/dockerfile:1.7

FROM node:24.18.0-trixie-slim AS node-deps

WORKDIR /app

COPY package.json package-lock.json* ./
RUN --mount=type=cache,target=/root/.npm,sharing=locked npm ci

FROM node-deps AS playwright-browsers

RUN npx playwright install chromium

FROM node-deps AS fe-unit-test

WORKDIR /app

COPY tsconfig*.json ./
COPY jest.config.cjs ./
COPY src ./src

RUN npm run test:unit

FROM rust:1.88-bookworm AS rust-builder-base

WORKDIR /workspace

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    set -eux; \
    rm -f /etc/apt/apt.conf.d/docker-clean; \
    printf 'Binary::apt::APT::Keep-Downloaded-Packages "true";\n' > /etc/apt/apt.conf.d/keep-cache; \
    apt-get update -o Acquire::Retries=5; \
    apt-get install -y -o Acquire::Retries=5 --no-install-recommends \
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
COPY src-tauri/devtools-capabilities ./src-tauri/devtools-capabilities
COPY src-tauri/icons ./src-tauri/icons
COPY src-tauri/keys ./src-tauri/keys
COPY src-tauri/tauri.conf.json ./src-tauri/tauri.conf.json

FROM rust-builder-base AS be-test-compile

RUN cargo test --manifest-path src-tauri/Cargo.toml --no-run --features cli-plane,ui-plane,desktop-updater

FROM be-test-compile AS be-unit-test

RUN cargo test --manifest-path src-tauri/Cargo.toml --features cli-plane,ui-plane,desktop-updater

FROM rust-builder-base AS app-build-cli-prod

RUN cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin fini --features cli-plane && \
    cp src-tauri/target/release/fini /workspace/fini && \
    rm -rf src-tauri/target

FROM rust-builder-base AS app-build-cli-dev

RUN cargo build --manifest-path src-tauri/Cargo.toml --locked --bin fini --features cli-plane && \
    cp src-tauri/target/debug/fini /workspace/fini && \
    rm -rf src-tauri/target

FROM rust-builder-base AS app-build-ui-dev

COPY --from=node-deps /usr/local/ /usr/local/

WORKDIR /workspace

ARG TAURI_CAPABILITIES_DIR=src-tauri/devtools-capabilities
COPY ${TAURI_CAPABILITIES_DIR} ./src-tauri/capabilities

COPY package.json package-lock.json* ./
COPY --from=node-deps /app/node_modules ./node_modules
COPY tsconfig*.json ./
COPY src ./src
COPY vite.config.ts ./
COPY index.html ./

RUN npm run tauri -- build --debug --features ui-plane,devtools --no-bundle -- --bin fini-app && \
    test -x src-tauri/target/debug/fini-app && \
    cp src-tauri/target/debug/fini-app /workspace/fini-app && \
    rm -rf src-tauri/target

FROM ubuntu:24.04 AS runtime-base

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    set -eux; \
    rm -f /etc/apt/apt.conf.d/docker-clean; \
    printf 'Binary::apt::APT::Keep-Downloaded-Packages "true";\n' > /etc/apt/apt.conf.d/keep-cache; \
    apt-get update -o Acquire::Retries=5; \
    apt-get install -y -o Acquire::Retries=5 --no-install-recommends --fix-missing \
      ca-certificates \
      libasound2t64 \
      libayatana-appindicator3-1 \
      libgtk-3-0 \
      libwebkit2gtk-4.1-0 \
      librsvg2-2; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

ENV XDG_DATA_HOME=/data

VOLUME ["/data"]

FROM runtime-base AS runtime

COPY --from=app-build-cli-prod /workspace/fini /usr/local/bin/fini

ENTRYPOINT ["/usr/local/bin/fini"]

FROM node-deps AS dev-runner

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    set -eux; \
    rm -f /etc/apt/apt.conf.d/docker-clean; \
    printf 'Binary::apt::APT::Keep-Downloaded-Packages "true";\n' > /etc/apt/apt.conf.d/keep-cache; \
    apt-get update -o Acquire::Retries=5; \
    apt-get install -y -o Acquire::Retries=5 --no-install-recommends --fix-missing \
      ca-certificates \
      libasound2 \
      libayatana-appindicator3-1 \
      libgtk-3-0 \
      libwebkit2gtk-4.1-0 \
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
      fonts-liberation \
      xvfb \
      xauth; \
    rm -rf /var/lib/apt/lists/*

COPY --from=playwright-browsers /root/.cache/ms-playwright /root/.cache/ms-playwright

COPY --from=app-build-ui-dev /workspace/fini-app /usr/local/bin/fini-app
COPY --from=app-build-cli-dev /workspace/fini /usr/local/bin/fini
COPY --from=app-build-ui-dev /workspace/dist ./dist

WORKDIR /app

COPY package.json package-lock.json* ./
COPY --from=node-deps /app/node_modules ./node_modules
COPY tsconfig*.json ./
COPY src ./src
COPY vite.config.ts ./
COPY index.html ./
COPY specs/e2e ./specs/e2e
COPY scripts/e2e-runner.sh ./scripts/e2e-runner.sh

ENV FINI_E2E_SOCKET_DIR=/var/run/fini-e2e \
    FINI_E2E_ROOT=/app/test-results/fini-e2e-runs \
    FINI_APP_BINARY=/usr/local/bin/fini-app \
    FINI_CLI_BINARY=/usr/local/bin/fini \
    FINI_E2E_CONTAINER_RUNNER=1 \
    TZ=UTC

CMD ["sh", "./scripts/e2e-runner.sh"]
