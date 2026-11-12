#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs gource with a time limit and its stderr in a file. render-inner.sh starts
# it through xvfb-run:
#
#   xvfb-run ... gource-run.sh <timeout-seconds> <stderr-file> <gource args...>
#
# Why not redirect around xvfb-run instead: xvfb-run runs its command with
# `2>&1`, so anything the command writes to stderr (a gource error, a Mesa
# warning) would be spliced into the PPM stream on stdout and corrupt it.
# Redirecting stderr here, inside the command, happens before that merge.
timeout_seconds=$1
stderr_file=$2
shift 2

exec timeout -k 10 "$timeout_seconds" gource "$@" 2>>"$stderr_file"
