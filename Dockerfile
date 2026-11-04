# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# nestwatch-server with the browser listener (/app/) and the website (/),
# ready for docker compose. See docs/self-hosting.md and packaging/docker/.
#
#   docker build -t nestwatch-server .
#
# Published for linux/amd64 and linux/arm64 as
# ghcr.io/jeroentrappers/nestwatch-server (.github/workflows/docker.yml).

ARG RUST_IMAGE=rust:1-bookworm

# ---- browser listener and website: architecture independent ---------------
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS web
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown
WORKDIR /src
# wasm-bindgen-cli must match the wasm-bindgen version in Cargo.lock.
COPY Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    v=$(awk '$0 == "name = \"wasm-bindgen\"" { getline; gsub(/"/, "", $3); print $3; exit }' Cargo.lock) \
 && cargo install --locked wasm-bindgen-cli --version "$v"
COPY . .
# Use the image's toolchain rather than letting rust-toolchain.toml fetch one.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=nestwatch-web-target \
    export RUSTUP_TOOLCHAIN=$(rustup default | cut -d' ' -f1) \
 && sh apps/web/build.sh /out/app \
 && sh apps/site/build.sh /out/site

# ---- server binary: built natively for each target platform ---------------
FROM ${RUST_IMAGE} AS server
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=nestwatch-server-target-$TARGETARCH \
    export RUSTUP_TOOLCHAIN=$(rustup default | cut -d' ' -f1) \
 && CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked -p nestwatch-server \
 && install -D target/release/nestwatch-server /out/nestwatch-server \
 && install -d -o 65532 -g 65532 /out/data

# ---- runtime ---------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=server /out/nestwatch-server /usr/local/bin/nestwatch-server
COPY --from=web /out/app /srv/nestwatch/app
COPY --from=web /out/site /srv/nestwatch/site
COPY --from=server --chown=65532:65532 /out/data /data
ENV NESTWATCH_SERVER_HTTP=0.0.0.0:8740 \
    NESTWATCH_SERVER_STUN=0.0.0.0:3478 \
    NESTWATCH_SERVER_STATE_DIR=/data \
    NESTWATCH_SERVER_APP_DIR=/srv/nestwatch/app \
    NESTWATCH_SERVER_SITE_DIR=/srv/nestwatch/site
VOLUME /data
EXPOSE 8740/tcp 3478/udp
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/nestwatch-server"]
LABEL org.opencontainers.image.title="nestwatch-server" \
      org.opencontainers.image.description="Nestwatch rendezvous and relay server with the browser listener and website" \
      org.opencontainers.image.source="https://github.com/jeroentrappers/nestwatch" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"
