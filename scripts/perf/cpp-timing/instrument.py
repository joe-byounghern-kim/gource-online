#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Add per-frame logic()/draw() timing to a *copy* of src/gource.cpp.

    instrument.py <path/to/copy/of/src/gource.cpp>

Every Gource::update writes one line to $GOURCE_TIMING_FILE (if set):
    frame logic_us draw_us finish_us files users dirs
with std::chrono::steady_clock. GOURCE_TIMING_FINISH=1 adds glFinish()
after draw() (finish_us), so GPU completion is timed too.
"""
import sys

path = sys.argv[1]
src = open(path, encoding="utf-8").read()

old_include = '#include "gource.h"\n'
new_include = old_include + '''#include <chrono>
#include <cstdio>
#include <cstdlib>

static FILE* gource_timing_file() {
    static FILE* f = 0;
    static bool init = false;
    if(!init) {
        init = true;
        const char* p = getenv("GOURCE_TIMING_FILE");
        if(p && *p) f = fopen(p, "w");
    }
    return f;
}
'''
old_block = '''    logic_time = SDL_GetTicks();

    logic(runtime, scaled_dt);

    logic_time = SDL_GetTicks() - logic_time;

    draw(runtime, scaled_dt);
'''
new_block = '''    logic_time = SDL_GetTicks();

    auto timing_t0 = std::chrono::steady_clock::now();

    logic(runtime, scaled_dt);

    auto timing_t1 = std::chrono::steady_clock::now();

    logic_time = SDL_GetTicks() - logic_time;

    draw(runtime, scaled_dt);

    auto timing_t2 = std::chrono::steady_clock::now();
    static const bool timing_finish = getenv("GOURCE_TIMING_FINISH") && *getenv("GOURCE_TIMING_FINISH") == '1';
    if(timing_finish) glFinish();
    auto timing_t3 = std::chrono::steady_clock::now();
    if(FILE* tf = gource_timing_file()) {
        auto us = [](std::chrono::steady_clock::duration d) {
            return (long long) std::chrono::duration_cast<std::chrono::microseconds>(d).count();
        };
        fprintf(tf, "%d %lld %lld %lld %d %d %d\\n", framecount,
            us(timing_t1 - timing_t0), us(timing_t2 - timing_t1), us(timing_t3 - timing_t2),
            (int) files.size(), (int) users.size(), (int) gGourceDirMap.size());
    }
'''
for old, new in ((old_include, new_include), (old_block, new_block)):
    if src.count(old) != 1:
        sys.exit(f"instrument.py: anchor not found exactly once in {path}:\n{old}")
    src = src.replace(old, new)
open(path, "w", encoding="utf-8").write(src)
