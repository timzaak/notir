# Frontend build stage
FROM node:20 AS frontend_builder
WORKDIR /app/frontend
COPY frontend/ ./
RUN npm install
RUN npm run build

# Rust builder stage
FROM rust:1.87-alpine3.22 AS builder
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
