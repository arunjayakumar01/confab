# Build from the workspace root: docker build -f services/google-twin/Dockerfile .
FROM rust:1.91-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p google-twin --recipe-path recipe.json
COPY . .
RUN cargo build --release -p google-twin

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 prism \
 && mkdir -p /data && chown prism /data
COPY --from=builder /app/target/release/google-twin /usr/local/bin/google-twin
USER prism
WORKDIR /data
EXPOSE 8081
CMD ["google-twin"]
