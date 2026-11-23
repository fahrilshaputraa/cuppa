# syntax=docker/dockerfile:1
FROM rust:1.96-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home /app cuppa && mkdir -p /app/data && chown cuppa /app/data
WORKDIR /app
COPY --from=build /src/target/release/cuppa /app/cuppa
COPY static /app/static
USER cuppa
ENV CUPPA_BIND=0.0.0.0:8080 DATABASE_URL=sqlite:///app/data/cuppa.db
EXPOSE 8080
VOLUME /app/data
ENTRYPOINT ["/app/cuppa"]
