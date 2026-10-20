# lisek-server: release build + FFmpeg runtime.
#
# Build context is the folder that holds LisekMovie-app AND the provider
# plugins (Cargo path dependencies `../../../LisekMovie-plugin-*`), e.g.
# D:\Projekty\Lisek. docker-compose.yml sets this up; by hand:
#
#   docker build -f LisekMovie-app/LisekMovie-Backend/Dockerfile -t lisekmovie/server ..
#
# What the context may contain is listed in Dockerfile.dockerignore next to
# this file (an allowlist: only the backend and the two plugins are sent).
#
# FFmpeg: `FFMPEG=jellyfin` (default) installs jellyfin-ffmpeg from the
# Jellyfin APT repository (NVENC/NVDEC, tonemap_cuda, OpenCL, libplacebo,
# VAAPI/QSV); `FFMPEG=debian` takes the distribution's ffmpeg (software and
# NVENC only, no tonemap_cuda).

ARG RUST_VERSION=1.97
ARG DEBIAN_RELEASE=trixie

# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config cmake perl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY LisekMovie-plugin-tmdb/ LisekMovie-plugin-tmdb/
COPY LisekMovie-plugin-thetvdb/ LisekMovie-plugin-thetvdb/
COPY LisekMovie-app/LisekMovie-Backend/ LisekMovie-app/LisekMovie-Backend/

WORKDIR /src/LisekMovie-app/LisekMovie-Backend
# Queries are checked against the committed `.sqlx` data, not a live database.
# Release tuning for the image only (Cargo.toml keeps fast local builds).
ENV SQLX_OFFLINE=true \
    CARGO_TERM_COLOR=never \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4 \
    CARGO_PROFILE_RELEASE_STRIP=symbols

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/LisekMovie-app/LisekMovie-Backend/target,sharing=locked \
    cargo build --release --locked -p lisek-server \
    && install -m 0755 target/release/lisek-server /usr/local/bin/lisek-server

# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime

ARG FFMPEG=jellyfin
ARG JELLYFIN_FFMPEG_PACKAGE=jellyfin-ffmpeg7

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl tini tzdata; \
    if [ "$FFMPEG" = "jellyfin" ]; then \
        apt-get install -y --no-install-recommends gnupg; \
        install -d -m 0755 /etc/apt/keyrings; \
        curl -fsSL https://repo.jellyfin.org/jellyfin_team.gpg.key \
            | gpg --dearmor -o /etc/apt/keyrings/jellyfin.gpg; \
        . /etc/os-release; \
        printf 'Types: deb\nURIs: https://repo.jellyfin.org/debian\nSuites: %s\nComponents: main\nArchitectures: %s\nSigned-By: /etc/apt/keyrings/jellyfin.gpg\n' \
            "$VERSION_CODENAME" "$(dpkg --print-architecture)" > /etc/apt/sources.list.d/jellyfin.sources; \
        apt-get update; \
        apt-get install -y --no-install-recommends "$JELLYFIN_FFMPEG_PACKAGE"; \
        ln -s /usr/lib/jellyfin-ffmpeg/ffmpeg /usr/local/bin/ffmpeg; \
        ln -s /usr/lib/jellyfin-ffmpeg/ffprobe /usr/local/bin/ffprobe; \
        apt-get purge -y gnupg; \
    else \
        apt-get install -y --no-install-recommends ffmpeg; \
    fi; \
    # NVIDIA OpenCL (tonemap_opencl): the container toolkit mounts the
    # library but no ICD file.
    install -d /etc/OpenCL/vendors; \
    echo "libnvidia-opencl.so.1" > /etc/OpenCL/vendors/nvidia.icd; \
    apt-get autoremove -y; \
    rm -rf /var/lib/apt/lists/*; \
    ffmpeg -hide_banner -version | head -n 1

# Unprivileged user. `video`/`render` let it open /dev/dri for VAAPI/QSV
# when the device is passed in (the render group id differs per host:
# docker-compose.gpu.yml adds it with `group_add`).
RUN groupadd --system --gid 1000 lisek \
    && useradd --system --uid 1000 --gid lisek --groups video --home-dir /var/lib/lisekmovie --shell /usr/sbin/nologin lisek \
    && install -d -o lisek -g lisek /var/lib/lisekmovie /var/lib/lisekmovie/config /media

COPY --from=build /usr/local/bin/lisek-server /usr/local/bin/lisek-server

# The working directory is the data volume: the server keeps its `.env`
# (a generated LISEK_SECRET_KEY) there, and LISEK_CONFIG_DIR holds the image
# cache, trickplay sheets and HLS transcodes.
WORKDIR /var/lib/lisekmovie
ENV LISEK_BIND_ADDR=0.0.0.0:8080 \
    LISEK_CONFIG_DIR=/var/lib/lisekmovie/config \
    LISEK_FFMPEG=/usr/local/bin/ffmpeg \
    LISEK_FFPROBE=/usr/local/bin/ffprobe \
    RUST_LOG=info
USER lisek
EXPOSE 8080
VOLUME ["/var/lib/lisekmovie"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

# tini: PID 1 that forwards signals and reaps FFmpeg processes.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/usr/local/bin/lisek-server"]
