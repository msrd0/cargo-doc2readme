# we don't actually need the cross compiler, but this image happens to include the latest
# stable rust compiler so we'll make use of that
# alpine's edge branch unfortunately sometimes trails behind, be it because firefox doesn't
# like building on the latest stable, or because they are waiting for another stable version
# to branch off first
FROM ghcr.io/msrd0/abuild-aarch64 AS builder

USER root
RUN apk add --no-cache cargo curl-dev

COPY Cargo.toml /src/
COPY Cargo.lock /src/
COPY src/ /src/src
WORKDIR /src/
RUN cargo build --release --locked

# start in a clean alpine so that we don't include the crates.io registry and other large
# files in the final image
FROM alpine

RUN apk add --no-cache cargo libcurl
COPY --from=builder /src/target/release/cargo-doc2readme /usr/bin/
