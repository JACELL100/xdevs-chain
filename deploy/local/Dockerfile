# syntax=docker/dockerfile:1
#
# xDEVS Chain node image: xdevs (node), xdevs-cli (keys, testnet init, transfers) and
# xdevs-loadgen. Multi-stage: cargo-chef caches the dependency build in its own layer, so
# a source-only change rebuilds just the workspace crates.
#
# Build from the repo root (linux/amd64 and linux/arm64; CI builds each natively):
#   docker build -f deploy/local/Dockerfile -t xdevs:local .
#
# deploy/local/Dockerfile.dockerignore trims the build context (BuildKit reads the
# Dockerfile-specific ignore file); it also keeps out the local-only .cargo/config.toml
# and rust-toolchain.toml, so the image's pinned Rust is used instead of "stable".

ARG RUST_VERSION=1.99
ARG DEBIAN_SUITE=trixie

# ---- chef: Rust toolchain + cargo-chef ---------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_SUITE} AS chef
ARG CARGO_CHEF_VERSION=^0.1
ENV CARGO_NET_RETRY=10 \
    CARGO_TERM_COLOR=never
RUN cargo install cargo-chef --locked --version "${CARGO_CHEF_VERSION}" \
 && rm -rf "${CARGO_HOME}/registry"
WORKDIR /src

# ---- planner: dependency recipe (changes only when manifests / Cargo.lock change) --
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- builder ------------------------------------------------------------------------
FROM chef AS builder
ARG PACKAGES="xdevs xdevs-cli xdevs-loadgen"
COPY --from=planner /src/recipe.json recipe.json
# Dependencies only: this layer is reused until the recipe changes.
RUN set -eu; pkgs=""; for p in ${PACKAGES}; do pkgs="$pkgs --package $p"; done; \
    cargo chef cook --release --locked --recipe-path recipe.json $pkgs
COPY . .
RUN set -eu; pkgs=""; for p in ${PACKAGES}; do pkgs="$pkgs --package $p"; done; \
    cargo build --release --locked $pkgs; \
    mkdir -p /out; \
    for p in ${PACKAGES}; do cp "target/release/$p" /out/; done; \
    /out/xdevs --version; /out/xdevs-cli --version

# ---- runtime ------------------------------------------------------------------------
FROM debian:${DEBIAN_SUITE}-slim AS runtime
ARG XDEVS_UID=10001
LABEL org.opencontainers.image.title="xdevs-chain" \
      org.opencontainers.image.description="xDEVS Chain node (xdevs), CLI (xdevs-cli) and load generator (xdevs-loadgen)" \
      org.opencontainers.image.source="https://github.com/JACELL100/xdevs-chain" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="THE xDEVS"
# curl: compose healthchecks and scripts/check.sh (JSON-RPC over docker exec).
# tini: PID 1 that forwards signals and reaps zombies.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid "${XDEVS_UID}" xdevs \
 && useradd --system --uid "${XDEVS_UID}" --gid xdevs --home-dir /var/lib/xdevs \
      --no-create-home --shell /usr/sbin/nologin xdevs \
 && mkdir -p /var/lib/xdevs /localnet \
 && chown xdevs:xdevs /var/lib/xdevs /localnet
COPY --from=builder /out/ /usr/local/bin/
USER xdevs:xdevs
WORKDIR /var/lib/xdevs
# JSON-RPC (HTTP + WS) of `xdevs run --dev`; localnet nodes use the ports in their config.
EXPOSE 8545
# The node shuts down gracefully on Ctrl-C (SIGINT), not on SIGTERM.
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/bin/tini", "--", "xdevs"]
# Default: a single-node dev chain. Override with e.g. `run --config /path/config.toml`.
CMD ["run", "--dev", "--http", "0.0.0.0:8545", "--data-dir", "/var/lib/xdevs"]
