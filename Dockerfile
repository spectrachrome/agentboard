# syntax=docker/dockerfile:1

# ---- web ui -----------------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# ---- rust: cargo-chef caches compiled dependencies --------------------------
# The base images are pinned to the same Debian release so the binary built
# here runs against the glibc in the runtime image.
FROM rust:1-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Rebuilt only when dependencies change.
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# ---- runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin agentboard \
 && mkdir /data && chown agentboard /data
COPY --from=builder /app/target/release/agentboard /usr/local/bin/agentboard
COPY --from=web /web/dist /srv/web

ENV AGENTBOARD_DB=/data/agentboard.db \
    AGENTBOARD_AGENT_ADDR=0.0.0.0:7777 \
    AGENTBOARD_HUMAN_ADDR=0.0.0.0:7778 \
    AGENTBOARD_WEB_DIR=/srv/web

USER agentboard
VOLUME /data
# 7777: agents (MCP shims, pi extension). 7778: humans (web UI). Publish them separately.
EXPOSE 7777 7778
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s CMD ["agentboard", "health"]
ENTRYPOINT ["agentboard"]
CMD ["serve"]
