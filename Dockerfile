# Frontend build stage
FROM node:20 AS frontend_builder
WORKDIR /app/frontend
COPY frontend/ ./
RUN npm install
RUN npm run build

# Rust builder stage
FROM rust:1.89-alpine3.22 AS builder
RUN apk add --no-cache musl-dev make
WORKDIR /app
COPY . .
COPY --from=frontend_builder /app/frontend/dist ./static
RUN cargo build --release

# Final image
FROM alpine:3.22
WORKDIR /app
COPY --from=builder /app/target/release/notir .
EXPOSE 5800
CMD ["./notir"]
