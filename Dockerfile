FROM rust:1.100-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
COPY --from=build /src/target/release/mech-bot /usr/local/bin/mech-bot
ENTRYPOINT ["/usr/local/bin/mech-bot"]
