ARG RUST_VERSION=1.95.0
FROM rust:${RUST_VERSION} AS toolchain

WORKDIR /app

RUN cargo install cargo-chef --locked

FROM toolchain AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM toolchain AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --locked --release -p agbranch

# Build-only artifact stage.
# Example:
#   docker build -f docker/build.Dockerfile --output type=local,dest=dist .
FROM scratch AS artifact
COPY --from=builder /app/target/release/agbranch /agbranch
