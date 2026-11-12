# SPDX-License-Identifier: GPL-3.0-or-later
#
# The C++ Gource with per-frame logic()/draw() timing (instrument.py applied
# to a temporary copy of the sources by build.sh). Reuses the reference image
# for the toolchain, Xvfb and Mesa.
FROM gource-reference:0.57
ARG MAKE_JOBS=8
WORKDIR /build
COPY . /build
RUN autoreconf -f -i \
 && ./configure --prefix=/opt/gource-timing \
 && make -j"${MAKE_JOBS}" \
 && make install-strip \
 && cd / && rm -rf /build
ENV PATH=/opt/gource-timing/bin:${PATH}
WORKDIR /tmp
