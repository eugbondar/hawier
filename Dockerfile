# syntax=docker/dockerfile:1

# Pinned, in step with rust-toolchain.toml: a floating tag drifts away from the
# host and every drift mints a second full copy of target/.
#
# Fully qualified, here and in both runtime stages: podman with
# short-name-mode = enforcing (the default on several distributions) refuses to
# resolve a bare `rust:1.97-alpine` without a TTY to ask on, and a build has no
# TTY. Docker normalises docker.io/library/... to exactly the same image, so
# this costs it nothing.
FROM docker.io/library/rust:1.97-alpine AS base
RUN apk add --no-cache build-base pkgconfig
# A property of the toolchain, not of the source: its own layer, so a changed
# line of Rust does not re-download a target.
RUN rustup target add wasm32-wasip1
WORKDIR /app
# scripts/ before the crates: every build.rs does `include!` on
# scripts/build_version.rs.
COPY Cargo.toml Cargo.lock ./
COPY scripts ./scripts
COPY apps ./apps
COPY libs ./libs

# The build context has no .git, so the commit is passed in (see `make
# docker`); every binary stamps it into HAWIER_BUILD. Declared here rather
# than at the top of the stage: as an ENV above the COPYs it changed on every
# commit and invalidated them too.
ARG GIT_SHA
ARG BUILD_DATE
ENV GIT_SHA=${GIT_SHA}
ENV BUILD_DATE=${BUILD_DATE}

# Which cargo profile the image is built with. `dev` by default, because the
# overwhelming majority of builds here exist so someone can look at a page:
# release spends minutes on optimisation and thin LTO that nobody watching a
# console will notice. `release` is for the thing that measures the binary —
# `make load` refuses to record a run against anything else — and for
# anything that leaves this machine.
ARG CARGO_PROFILE=dev

# Cargo's registry and target directory are caches, not build outputs. Baked
# into the image layer they are thrown away whenever a source file changes,
# so a one-line edit recompiled wasmtime, cranelift and sqlx from zero —
# six to eight minutes on every deploy. As cache mounts they persist across
# builds and only what actually changed is rebuilt.
#
# Two consequences of a cache mount, both load-bearing here:
#  - it is not part of the resulting layer, so anything built inside it has
#    to be copied out within the same RUN or the next stage finds nothing;
#  - it is shared, and an unnamed one is keyed by target path alone — so it
#    is shared with anything else on the machine that ever mounted a cache at
#    /usr/local/cargo/registry. That is not hypothetical: the first attempt
#    inherited another project's index and cargo refused to resolve socket2,
#    offering a 0.4 prerelease for a 0.6 requirement. Hence the explicit ids.
#    `sharing=locked` on top, because compose builds the server and console
#    images from this same stage and they would otherwise write one target
#    directory at once.
RUN --mount=type=cache,id=hawier-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=hawier-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=hawier-target,target=/app/target,sharing=locked \
    set -eu \
 && case "$CARGO_PROFILE" in dev) OUT=debug ;; *) OUT="$CARGO_PROFILE" ;; esac \
 && cargo build --profile "$CARGO_PROFILE" --workspace \
 && cargo build --profile "$CARGO_PROFILE" --target wasm32-wasip1 \
      -p hawier-ext-fs -p hawier-ext-s3 -p hawier-ext-webhook -p hawier-ext-retention \
      -p hawier-ext-rollout-classic -p hawier-ext-rollout-bucket -p hawier-ext-console \
      -p hawier-ext-authz \
 && mkdir -p /out/bin /out/wasm \
 && cp "target/$OUT/hawier" "target/$OUT/hawier-ui" /out/bin/ \
 && cp target/wasm32-wasip1/"$OUT"/*.wasm /out/wasm/

# --- hawier server (hawkBit rewrite) ---------------------------------------
FROM docker.io/library/alpine:3.20 AS server
RUN addgroup -S hawier && adduser -S hawier -G hawier \
    && mkdir -p /artifacts \
    && chown -R hawier:hawier /artifacts
COPY --from=base /out/bin/hawier /usr/local/bin/hawier
# hawier-device is deliberately NOT here. It is a measuring instrument of the
# developer's machine: scripts/load.sh and scripts/load_poll.sh default to
# target/release/hawier-device on the host, and gc.sh keeps that one alive on
# purpose. Nothing under deploy/ ever ran it from the image -- no service, no
# command, no variable -- so it rode to every node and sat there: 7 MB of the
# release image, and three copies per host under hawier_keep_images.
# Extensions are wasm modules; a node with no storage extension serves polls
# and deployments but answers 503 for artifacts, so ship them alongside.
COPY --from=base /out/wasm/ /usr/local/lib/hawier/
ENV HAWIER_BIND_ADDR=0.0.0.0:8080
ENV HAWIER_ARTIFACT_DIR=/artifacts
ENV RUST_LOG=hawier_server=info,tower_http=info
# Extension parameters live here; the volume takes its ownership from the
# image, and the node runs unprivileged.
RUN mkdir -p /etc/hawier && chown hawier:hawier /etc/hawier

USER hawier
EXPOSE 8080
CMD ["hawier"]

# --- hawier UI -------------------------------------------------------------
FROM docker.io/library/alpine:3.20 AS ui
RUN addgroup -S hawier && adduser -S hawier -G hawier \
    && mkdir -p /var/lib/hawier-ui \
    && chown -R hawier:hawier /var/lib/hawier-ui
WORKDIR /app
COPY --from=base /out/bin/hawier-ui /usr/local/bin/hawier-ui
COPY static ./static
ENV HAWIER_BIND_ADDR=0.0.0.0:8088
ENV HAWIER_STATIC_DIR=/app/static
ENV HAWIER_DATA_DIR=/var/lib/hawier-ui
ENV RUST_LOG=hawier_ui=info,tower_http=info

USER hawier
EXPOSE 8088
CMD ["hawier-ui"]
