# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.98
ARG NODE_VERSION=22
ARG PYTHON_VERSION=3.14
ARG LOGIKSMITH_FEATURES

FROM node:${NODE_VERSION}-bookworm-slim AS dashboard
WORKDIR /src/logiksmith-web
COPY logiksmith-web/package.json logiksmith-web/package-lock.json ./
RUN npm ci
COPY logiksmith-web/ ./
RUN npm run build

FROM rust:${RUST_VERSION}-bookworm AS rust-build
ARG LOGIKSMITH_FEATURES
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN if [ -n "${LOGIKSMITH_FEATURES:-}" ]; then \
        cargo build --release -p logiksmith-desktop --no-default-features --features "${LOGIKSMITH_FEATURES}"; \
    else \
        cargo build --release -p logiksmith-desktop; \
    fi
COPY --from=dashboard /src/logiksmith-web/dist/ logiksmith-web/dist/

# The desktop host is the foreground process. Docker (or another external
# supervisor) owns restart policy and replacement after a fatal bridge error.
FROM python:${PYTHON_VERSION}-slim-bookworm AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    LOGIKSMITH_CONFIG_PATH=/config/local.toml \
    LOGIKSMITH_AUTOMATION_PATH=/config/automation.toml \
    LOGIKSMITH_RUNTIME_PROFILE=desktop \
    RUST_LOG=info

RUN groupadd --system --gid 10001 logiksmith \
    && useradd --system --uid 10001 --gid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin logiksmith \
    && mkdir -p /app/logiksmith-web/dist /config \
    && chown -R logiksmith:logiksmith /app /config

WORKDIR /app

COPY --chown=logiksmith:logiksmith bridges/xknx/ /app/bridges/xknx/
RUN python -m pip install --no-cache-dir --requirement /app/bridges/xknx/requirements.txt \
    && python -m pip install --no-cache-dir --no-deps /app/bridges/xknx/

COPY --from=rust-build --chown=logiksmith:logiksmith /src/target/release/logiksmith-desktop /usr/local/bin/logiksmith
COPY --from=rust-build --chown=logiksmith:logiksmith /src/logiksmith-web/dist/ /app/logiksmith-web/dist/

COPY --chown=root:root docker/entrypoint.sh /usr/local/bin/logiksmith-entrypoint
RUN chmod 0555 /usr/local/bin/logiksmith-entrypoint

USER logiksmith:logiksmith
VOLUME ["/config"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["python", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/readyz', timeout=2)"]

ENTRYPOINT ["/usr/local/bin/logiksmith-entrypoint"]
CMD []
