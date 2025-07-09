FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /app/target/release/notir .
EXPOSE 5800
CMD ["./notir"]