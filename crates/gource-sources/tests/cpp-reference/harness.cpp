// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.
//
// Reference harness for the Rust custom-log parser (crates/gource-sources).
//
// It runs the custom-log reading logic of Gource (formats/custom.cpp,
// formats/commitlog.cpp, core/seeklog.cpp) over arbitrary byte strings and
// prints what the parser would hand to Gource::processCommit. Everything that
// does not need SDL or OpenGL is the real thing: the PCRE2 `custom_regex`
// through the unmodified core/regex.cpp, core/timezone.cpp, the bundled
// utf8cpp `replace_invalid`, and SDLAppSettings::parseDateTime (cut out of
// core/settings.cpp by regen.sh into parse_date_time.inc). The few SDL-bound
// pieces are transcribed from their sources and marked TRANSCRIBED below.
//
// Build and run through regen.sh. Protocol: stdin is a sequence of records
// "<decimal length>\n<bytes>", or with --raw a single blob. For every record
// the harness prints
//
//     C <timestamp> <hex username> <file count> <byte offset>   one per commit
//     F <A|M|D> <hex file name> <RRGGBB|->                      one per file
//     S <lines> <skipped lines> <commits> <files>
//     END
//
// The byte offset is where the commit's first line starts in the input. Gource
// does not record it; here it is the stream position (tellg) taken just before
// the getline that read that line.
//
// One deliberate normalisation: Gource's readLog loop drops a final line
// that has no newline when it starts a new commit (the stream's eof flag
// ends the loop before the saved line is read). The port parses that line, so
// the harness adds the missing newline first.

#include "regex.h"
#include "timezone.h"
#include "utf8/utf8.h"

#include <list>
#include <string>
#include <vector>
#include <sstream>
#include <iostream>
#include <iterator>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>

#include "parse_date_time.inc"

// ---------------------------------------------------------------------------
// formats/custom.cpp: the regex, verbatim.
// ---------------------------------------------------------------------------
static Regex custom_regex("^(?:\\xEF\\xBB\\xBF)?([^|]+)\\|([^|]*)\\|([ADM]?)\\|([^|]+)(?:\\|#?([a-fA-F0-9]{6}))?");

struct Vec3 { float x = 0, y = 0, z = 0; };

// ---------------------------------------------------------------------------
// formats/commitlog.cpp: filter_utf8, verbatim.
// ---------------------------------------------------------------------------
static std::string filter_utf8(const std::string& str) {
    std::string filtered;
    try {
        utf8::replace_invalid(str.begin(), str.end(), back_inserter(filtered), '?');
    }
    catch(...) {
        filtered = "???";
    }
    return filtered;
}

struct CFile {
    std::string filename;
    std::string action;
    Vec3 colour;
    bool has_colour;

    CFile(const std::string& fn, const std::string& act, Vec3 c, bool hc) {
        this->filename = filter_utf8(fn);
        //prepend a root slash
        if(this->filename[0] != '/') {
            this->filename.insert(0, 1, '/');
        }
        this->action = act;
        this->colour = c;
        this->has_colour = hc;
    }
};

struct CCommit {
    time_t timestamp = 0;
    std::string username;
    long long offset = 0;
    std::list<CFile> files;
};

struct Stats {
    long lines = 0;
    long skipped = 0;
};

// ---------------------------------------------------------------------------
// TRANSCRIBED: SeekLog::getNextLine / isFinished (core/seeklog.cpp) over an
// istringstream, RCommitLog::getNextLine (lastline), CustomLog::parse*.
// ---------------------------------------------------------------------------
struct Log {
    std::istringstream stream;
    std::string lastline;
    long long lastline_offset = 0; // where the saved line started
    long long line_offset = 0;     // where the line being parsed started
    Stats stats;

    Log(const std::string& data) : stream(data) {}

    bool isFinished() {
        return stream.fail() || stream.eof();
    }

    bool seekGetNextLine(std::string& line) {
        //try and fix the stream
        if(isFinished()) stream.clear();

        long long pos = (long long) stream.tellg();

        std::getline(stream, line);

        //remove carriage returns
        if (line.size() > 0 && line[line.size()-1] == '\r') {
            line.resize(line.size() - 1);
        }

        if(stream.fail()) {
            return false;
        }

        stats.lines++;
        line_offset = pos;
        return true;
    }

    bool getNextLine(std::string& line) {
        if(!lastline.empty()) {
            line = lastline;
            lastline.clear();
            line_offset = lastline_offset;
            return true;
        }
        return seekGetNextLine(line);
    }

    Vec3 parseColour(const std::string& cstr) {
        Vec3 colour;
        int r,g,b;
        if(sscanf(cstr.c_str(), "%02x%02x%02x", &r, &g, &b) == 3) {
            colour.x = r; colour.y = g; colour.z = b;
            colour.x /= 255.0f; colour.y /= 255.0f; colour.z /= 255.0f;
        }
        return colour;
    }

    bool parseCommit(CCommit& commit) {
        while(parseCommitEntry(commit));
        return !commit.files.empty();
    }

    bool parseCommitEntry(CCommit& commit) {
        std::string line;
        std::vector<std::string> entries;

        if(!getNextLine(line)) return false;

        //custom line
        if(!custom_regex.match(line, &entries)) { stats.skipped++; return false; }

        time_t timestamp;

        // Allow timestamp to be a string
        if(entries[0].size() > 1 && entries[0].find("-", 1) != std::string::npos) {
            if(!ref::parseDateTime(entries[0], timestamp)) { stats.skipped++; return false; }
        } else {
            timestamp = (time_t) atoll(entries[0].c_str());
            if(!timestamp && entries[0] != "0") { stats.skipped++; return false; }
        }

        std::string username = (entries[1].size()>0) ? entries[1] : "Unknown";
        std::string action   = (entries[2].size()>0) ? entries[2] : "A";

        if(commit.files.empty()) {
            commit.timestamp = timestamp;
            commit.username  = username;
            commit.offset    = line_offset;
        } else {
            if(commit.timestamp != timestamp || commit.username  != username) {
                lastline = line;
                lastline_offset = line_offset;
                return false;
            }
        }

        bool has_colour = false;
        Vec3 colour;

        if(entries.size()>=5 && entries[4].size()>0) {
            has_colour = true;
            colour = parseColour(entries[4]);
        }

        // RCommit::addFile without file filters configured.
        commit.files.push_back(CFile(entries[3], action, colour, has_colour));

        return true;
    }

    // RCommitLog::nextCommit(commit, true) without user filters.
    bool nextCommit(CCommit& commit) {
        commit = CCommit();
        bool success = parseCommit(commit);
        if(!success) return false;
        commit.username = filter_utf8(commit.username);
        return !commit.files.empty();
    }
};

static std::string hex(const std::string& s) {
    static const char* d = "0123456789abcdef";
    std::string out;
    for(unsigned char c : s) { out += d[c >> 4]; out += d[c & 15]; }
    return out;
}

// Canonical dump:
//   C <timestamp> <hex username> <file count> <byte offset>
//   F <A|M|D> <hex path> <RRGGBB|->
//   S <lines> <skipped> <commits> <files>
//   END
static void dump(const std::string& data_in) {
    // Gource's own readLog loop would drop a final unterminated line that
    // starts a new commit (eofbit makes isFinished() true early). The port
    // deliberately does not reproduce that, so normalise the input.
    std::string data = data_in;
    if(!data.empty() && data[data.size()-1] != '\n') data += '\n';

    Log log(data);
    long commits = 0, files = 0;

    while(true) {
        if(log.lastline.empty() && log.isFinished()) break;

        CCommit c;
        if(!log.nextCommit(c)) continue;

        commits++;
        files += c.files.size();
        printf("C %lld %s %zu %lld\n", (long long) c.timestamp, hex(c.username).c_str(), c.files.size(), c.offset);
        for(const CFile& f : c.files) {
            char col[16] = "-";
            if(f.has_colour) {
                snprintf(col, sizeof col, "%02x%02x%02x",
                    (int)(f.colour.x * 255.0f + 0.5f),
                    (int)(f.colour.y * 255.0f + 0.5f),
                    (int)(f.colour.z * 255.0f + 0.5f));
            }
            printf("F %s %s %s\n", f.action.c_str(), hex(f.filename).c_str(), col);
        }
    }

    printf("S %ld %ld %ld %ld\nEND\n", log.stats.lines, log.stats.skipped, commits, files);
}

int main(int argc, char** argv) {
    std::ios::sync_with_stdio(false);
    std::string all((std::istreambuf_iterator<char>(std::cin)), std::istreambuf_iterator<char>());

    if(argc > 1 && std::string(argv[1]) == "--raw") {
        // single blob = all of stdin
        dump(all);
        return 0;
    }

    size_t pos = 0;
    while(pos < all.size()) {
        size_t nl = all.find('\n', pos);
        if(nl == std::string::npos) break;
        size_t len = strtoull(all.substr(pos, nl - pos).c_str(), 0, 10);
        pos = nl + 1;
        if(pos + len > all.size()) { fprintf(stderr, "truncated record\n"); return 2; }
        dump(all.substr(pos, len));
        pos += len;
    }
    return 0;
}
