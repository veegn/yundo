FROM node:20-bookworm-slim AS frontend-builder

WORKDIR /app

COPY package.json package-lock.json ./
COPY frontend/package.json frontend/package.json
RUN npm ci

COPY frontend ./frontend
RUN npm run build

FROM rust:1.87-bookworm AS rust-builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends clang pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY frontend ./frontend
RUN cargo build --release \
    && strip target/release/precision-proxy

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=rust-builder /app/target/release/precision-proxy /usr/local/bin/precision-proxy
COPY --from=frontend-builder /app/frontend/dist ./frontend/dist

EXPOSE 8080

ENV RUST_LOG=info

ENTRYPOINT ["precision-proxy"]
CMD ["--host", "0.0.0.0", "--port", "8080", "--cache-dir", "/tmp/cache", "--frontend-dist", "./frontend/dist"]
