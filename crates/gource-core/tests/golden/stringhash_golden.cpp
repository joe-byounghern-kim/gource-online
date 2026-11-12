// Golden-value generator for gource-core's hash module.
//
// The functions below are copied verbatim from Gource's Core library
// (src/core/stringhash.cpp and src/core/vectors.cpp,
// Copyright (c) 2009 Andrew Caudwell, BSD-3-Clause, see those files), with
// the global gStringHashSeed set per test row. The output is the table in
// stringhash_golden.tsv; see README.md for how to regenerate it.

#define GLM_FORCE_RADIANS
#define GLM_FORCE_CTOR_INIT
#define GLM_ENABLE_EXPERIMENTAL
#include <glm/glm.hpp>

#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using glm::vec2;
using glm::vec3;

int gStringHashSeed = 31;

// --- vectors.cpp ---
vec2 normalise(const vec2& v) {
    float l = glm::length(v);
    if(l > 0.0) return v / l;
    return v;
}

vec3 normalise(const vec3& v) {
    float l = glm::length(v);
    if(l > 0.0) return v / l;
    return v;
}

// --- stringhash.cpp ---
int stringHash(const std::string& str) {

    int val = 0;
    int n = str.size();

    for (int i = 0; i < n; i++) {
        val = val + str[i] * (gStringHashSeed^(n-i));
    }

     if(val<0) {
         val = -val;
     }

    return val;
}

vec2 vec2Hash(const std::string& str) {
    int hash = stringHash(str);

    int x = ((hash/7) % 255) - 127;
    int y = ((hash/3) % 255) - 127;

    vec2 v = normalise(vec2(x, y));

    return v;
}

vec3 vec3Hash(const std::string& str) {
    int hash = stringHash(str);

    int x = ((hash/7) % 255) - 127;
    int y = ((hash/3) % 255) - 127;
    int z = hash % 255;

    vec3 v = normalise(vec3(x, y, z));

    return v;
}

vec3 colourHash(const std::string& str) {
    int hash = stringHash(str);

    if(hash == 0) hash++;

    int r = (hash/7) % 255;
    int g = (hash/3) % 255;
    int b = hash % 255;

    vec3 colour = normalise(vec3(r, g, b));

    return colour;
}

// --- harness ---
static unsigned bits(float f) {
    unsigned u;
    std::memcpy(&u, &f, sizeof u);
    return u;
}

static std::string hex(const std::string& s) {
    static const char* digits = "0123456789abcdef";
    std::string out;
    for(unsigned char c : s) {
        out += digits[c >> 4];
        out += digits[c & 15];
    }
    return out.empty() ? "-" : out;
}

int main() {
    std::vector<std::string> strings = {
        "",
        "a",
        "c",
        "h",
        "cpp",
        "txt",
        "md",
        "rs",
        "Makefile",
        "README",
        "/",
        "/src/",
        "/src/core/",
        "/path/to/file/Not/",
        "/path/to/blah/",
        "bob",
        "Andrew Caudwell",
        "joe-byounghern-kim",
        // UTF-8 multibyte: bytes >= 0x80 are negative `char` on x86 g++.
        "\xc3\xa9",                                 // e acute
        "caf\xc3\xa9",                              // cafe
        "\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e",     // nihongo
        "/\xe3\x83\x87\xe3\x82\xa3\xe3\x83\xac\xe3\x82\xaf\xe3\x83\x88\xe3\x83\xaa/", // /direkutori/
        "\xf0\x9f\xa6\x80",                         // crab emoji (4 bytes)
        "J\xc3\xbcrgen M\xc3\xbcller",
        "\xd0\x9f\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82",
        // long strings: exercise signed overflow wrap-around.
        "/usr/share/doc/some/very/long/path/that/keeps/going/and/going/forever/file.extension",
        std::string(1000, 'z'),
        std::string(4096, '\xff'),
        std::string(333, '\x80') + "tail",
        "\x01",
        "\x80",
        "The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.",
    };

    std::vector<int> seeds = { 31, 0, 1, 2, 7, 37, 100, 255, 1000, 4097, 9999, 10000, -1, 2147483647, -2147483647, 16777217 };

    std::printf("# seed\tstring_hex\tstringHash\tvec2Hash.x\tvec2Hash.y\tvec3Hash.x\tvec3Hash.y\tvec3Hash.z\tcolourHash.r\tcolourHash.g\tcolourHash.b\n");

    for(int seed : seeds) {
        gStringHashSeed = seed;
        for(const std::string& s : strings) {
            vec2 v2 = vec2Hash(s);
            vec3 v3 = vec3Hash(s);
            vec3 c  = colourHash(s);
            std::printf("%d\t%s\t%d\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\n",
                seed, hex(s).c_str(), stringHash(s),
                bits(v2.x), bits(v2.y),
                bits(v3.x), bits(v3.y), bits(v3.z),
                bits(c.x), bits(c.y), bits(c.z));
        }
    }

    return 0;
}
