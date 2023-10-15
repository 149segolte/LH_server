FROM rust:1.72.1-slim-buster as build

RUN USER=root cargo new --bin sentry
WORKDIR sentry/

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release

RUN rm src/*.rs
COPY ./src ./src

RUN cargo build --release

FROM debian:buster-slim
COPY --from=build /sentry/target/release/sentry /opt/sentry

ENTRYPOINT ["/opt/sentry"]
