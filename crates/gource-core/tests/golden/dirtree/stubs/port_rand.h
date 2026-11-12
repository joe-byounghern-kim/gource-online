// Force-included into dirnode.cpp by build.sh: replaces rand() with the
// port's KeyedRng stream (src/hash.rs: keyed_u64 over seed 31 and the key
// "dirnode.applyForceDir"), so the overlap resolution is comparable bit for bit.
#pragma once
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <list>
#include <map>
#include <set>
#include <string>
#include <vector>

extern uint64_t port_rand_counter;
extern uint64_t port_rand_seed;

static inline uint64_t port_mix64(uint64_t z) {
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

static inline uint64_t port_stream_key(const char* s) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for(; *s; s++) {
        h ^= (unsigned char) *s;
        h *= 0x00000100000001b3ULL;
    }
    return h;
}

static inline int port_rand() {
    const uint64_t gamma = 0x9e3779b97f4a7c15ULL;
    uint64_t key  = port_stream_key("dirnode.applyForceDir");
    uint64_t base = port_mix64(port_rand_seed ^ port_mix64(key + gamma));
    uint64_t v    = port_mix64(base + (port_rand_counter + 1) * gamma);
    port_rand_counter++;
    return (int) (v >> 33);
}

#define rand port_rand
