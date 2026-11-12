// Force-included into dirnode.cpp and user.cpp by build.sh: replaces rand()
// with the port's KeyedRng streams (src/hash.rs: keyed_u64 over the seed and
// the key "<file>.<function>", one counter per key), so overlap resolution is
// comparable bit for bit. PORT_RAND_FILE is defined per translation unit.
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

extern uint64_t port_rand_seed;
extern std::map<std::string, uint64_t> port_rand_counters;

static inline uint64_t port_mix64(uint64_t z) {
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

static inline uint64_t port_stream_key(const std::string& s) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for(unsigned char c : s) {
        h ^= c;
        h *= 0x00000100000001b3ULL;
    }
    return h;
}

static inline int port_rand(const char* func) {
    const uint64_t gamma = 0x9e3779b97f4a7c15ULL;
    std::string name = std::string(PORT_RAND_FILE) + "." + func;
    uint64_t& counter = port_rand_counters[name];
    uint64_t key  = port_stream_key(name);
    uint64_t base = port_mix64(port_rand_seed ^ port_mix64(key + gamma));
    uint64_t v    = port_mix64(base + (counter + 1) * gamma);
    counter++;
    return (int) (v >> 33);
}

#define rand() port_rand(__func__)
