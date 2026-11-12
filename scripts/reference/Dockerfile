# SPDX-License-Identifier: GPL-3.0-or-later
#
# Reference renderer: the unmodified C++ Gource (this tree, v0.57) built on
# Ubuntu 24.04 together with everything needed to run it headless: Xvfb as the
# display and Mesa llvmpipe as the OpenGL implementation, so the output does not
# depend on any GPU or driver.
#
# The build context is assembled by scripts/reference/build.sh (src/, data/,
# m4/, configure.ac, Makefile.am); do not run `docker build` on the repository
# root.

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    TZ=UTC

# Build dependencies: everything configure.ac asks for. libgl-dev and
# libglu1-mesa-dev are what AX_CHECK_GL / AX_CHECK_GLU look for (libglew-dev
# pulls them in, listed anyway so the intent is visible).
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential autoconf automake libtool pkg-config \
        libsdl2-dev libsdl2-image-dev libpcre2-dev libfreetype-dev \
        libglew-dev libglm-dev libboost-filesystem-dev libpng-dev \
        libgl-dev libglu1-mesa-dev \
 && rm -rf /var/lib/apt/lists/*

# Runtime: virtual X display, software GL, python3 for the PPM -> PNG
# conversion (python3-pil is only used for the contact sheet), git because
# `gource --output-custom-log <repo>` runs `git log`.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        xvfb xauth libgl1-mesa-dri libglx-mesa0 mesa-utils \
        python3 python3-pil git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Build Gource exactly like the CLAUDE.md instructions: autoreconf, configure,
# make. `autoreconf -f -i` is the command autogen.sh runs; its remaining steps
# are `git submodule` calls, and build.sh already put src/core in the context.
# Installing into /opt/gource makes Gource find data/ (textures, shaders, font)
# under /opt/gource/share/gource, whatever the working directory is.
ARG MAKE_JOBS=8
WORKDIR /build
COPY . /build
RUN autoreconf -f -i \
 && ./configure --prefix=/opt/gource \
 && make -j"${MAKE_JOBS}" \
 && make install-strip \
 && cd / \
 && rm -rf /build

# Software GL, deterministic and cache-free.
ENV PATH=/opt/gource/bin:${PATH} \
    LIBGL_ALWAYS_SOFTWARE=1 \
    GALLIUM_DRIVER=llvmpipe \
    MESA_SHADER_CACHE_DISABLE=true \
    SDL_AUDIODRIVER=dummy \
    HOME=/tmp

WORKDIR /tmp

# Provenance (kept last so it never invalidates the compile layers).
ARG SOURCE_FINGERPRINT=unknown
LABEL org.opencontainers.image.title="gource-reference" \
      org.opencontainers.image.description="C++ Gource 0.57 with Xvfb and Mesa llvmpipe, for reference renders" \
      org.opencontainers.image.licenses="GPL-3.0-or-later" \
      gource.source-fingerprint="${SOURCE_FINGERPRINT}"

CMD ["gource", "--help"]
