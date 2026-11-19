# The registry's builder image (registry-repo.md, pr.yml step 2): the Rust toolchain Fyx pins,
# the wasm32-wasip2 target, and pinned `fyx-ext`, `fyx-registry` and `cargo-deny`. CI runs it
# with `--network none` for everything after `cargo fetch`.
#
# Build and push (owner, task 3.2), then put the pushed digest into policy/builder.toml:
#   docker build --build-arg FYX_COMMIT=<sha> -t ghcr.io/shelenhovskyi/fyx-ext-builder builder/
#   docker push ghcr.io/shelenhovskyi/fyx-ext-builder
# Every value below is pinned so the same arguments give the same tools.

# rust:1.98.1-slim-bookworm, by digest (fill in from `docker pull` when building).
FROM docker.io/library/rust:1.98.1-slim-bookworm

ARG FYX_REPOSITORY=https://github.com/shelenhovskyi/fyx
# The Fyx commit whose fyx-ext and fyx-registry the registry uses (policy/builder.toml fyx_ext).
ARG FYX_COMMIT
ARG CARGO_DENY_VERSION=0.21.0

RUN test -n "$FYX_COMMIT" \
    && apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates unzip \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2

RUN git init -q /fyx \
    && git -C /fyx fetch -q --depth 1 "$FYX_REPOSITORY" "$FYX_COMMIT" \
    && git -C /fyx checkout -q FETCH_HEAD \
    && cargo install --locked --root /usr/local --path /fyx/crates/fyx_ext \
    && cargo install --locked --root /usr/local --path /fyx/crates/fyx_registry \
    && cargo install --locked --root /usr/local cargo-deny --version "$CARGO_DENY_VERSION" \
    && rm -rf /fyx /usr/local/cargo/registry /usr/local/cargo/git

# Builds run as a plain user; the extension's build scripts get no root.
RUN useradd --create-home builder
USER builder
