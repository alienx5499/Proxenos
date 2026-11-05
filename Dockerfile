# Notary image and the Tier 1 test image. SQLite is compiled in (sqlx sqlite-bundled).
FROM rust:1.99.0-slim-trixie AS build
# cmake for aws-lc-sys (rustls crypto backend); gcc and libc headers ship with the image.
RUN apt-get update && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .

# `docker compose run --rm tests`: every deterministic claim, no Switch, no LLM.
FROM build AS tests
RUN cargo test --workspace --all-features --locked --no-run
CMD ["cargo", "test", "--workspace", "--all-features", "--locked"]

FROM build AS release
RUN cargo build --release --locked -p proxenos-notary

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin proxenos \
    && install -d -m 0700 -o proxenos -g proxenos /data
COPY --from=release /src/target/release/proxenos /usr/local/bin/proxenos
USER proxenos
WORKDIR /data
ENV PROXENOS_DB=/data/proxenos.db \
    PROXENOS_RECEIPT_KEY_FILE=/data/receipt.key \
    PROXENOS_ADDR=0.0.0.0:8080
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["proxenos"]
CMD ["serve"]
