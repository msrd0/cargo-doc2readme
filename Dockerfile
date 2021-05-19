FROM alpine AS builder

RUN echo "@edge https://dl-cdn.alpinelinux.org/alpine/edge/main" >>/etc/apk/repositories
RUN echo "@edge https://dl-cdn.alpinelinux.org/alpine/edge/community" >>/etc/apk/repositories
RUN apk add --no-cache cargo@edge curl-dev

# pre-compile for cache reuse
# I actually don't want the Cargo.lock file so that if it changes, only the dependency that changed needs to be
# updated, not all dependencies
RUN mkdir -p /src/src && echo 'fn main() {}' >/src/src/main.rs
COPY Cargo.toml /src/
WORKDIR /src/
RUN cargo build --release

COPY Cargo.lock /src/
COPY src/ /src/src
RUN touch src/main.rs \
 && cargo build --release --locked \
 && strip target/release/cargo-doc2readme

# start in a clean alpine so that we don't include the crates.io registry and other large files in the final image
FROM alpine

RUN apk add --no-cache cargo libcurl
COPY --from=builder /src/target/release/cargo-doc2readme /usr/bin/
