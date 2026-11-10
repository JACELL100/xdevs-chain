# Multi-stage build of the xDEVS Chain node on Linux.
# Build from the repo root:  docker build -f deploy/local/Dockerfile -t xdevs:dev .
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
# .cargo/config.toml is a local Windows dev setup (proxy, target dir); not for containers.
RUN rm -f .cargo/config.toml \
 && cargo build --release --locked -p xdevs -p xdevs-cli

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /var/lib/xdevs xdevs \
 && mkdir -p /var/lib/xdevs && chown xdevs:xdevs /var/lib/xdevs
COPY --from=builder /src/target/release/xdevs /src/target/release/xdevs-cli /usr/local/bin/
USER xdevs
VOLUME ["/var/lib/xdevs"]
EXPOSE 8545
ENTRYPOINT ["xdevs"]
CMD ["node", "--dev", "--data-dir", "/var/lib/xdevs"]
