# SPDX-License-Identifier: GPL-3.0-or-later
# Linux perf in a container: the host's kernel.perf_event_paranoid blocks
# unprivileged sampling, a container with CAP_PERFMON does not need it.
FROM ubuntu:24.04
RUN apt-get update -qq \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      linux-tools-generic binutils ca-certificates git perl \
 && ln -s "$(ls -d /usr/lib/linux-tools/*/ | head -1)perf" /usr/local/bin/perf \
 && git clone -q --depth 1 https://github.com/brendangregg/FlameGraph /opt/FlameGraph \
 && rm -rf /var/lib/apt/lists/*
