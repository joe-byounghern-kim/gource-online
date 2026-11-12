// Golden-value generator for gource-core's directory tree port.
//
// Compiles Gource's own dirnode.cpp, file.cpp, pawn.cpp and Core's quadtree,
// frustum, plane, stringhash and vectors against the no-op GL/SDL/font stubs
// in stubs/, then replays the scenario that src/tree_tests.rs replays
// (layout_matches_cpp_golden) and prints node and file state at fixed frames. See
// ../README.md for how to regenerate dirtree_golden.tsv.
//
// rand() in dirnode.cpp is replaced by the port's keyed stream (port_rand.h),
// the one intended difference from a stock Gource build.

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <list>
#include <map>
#include <set>
#include <string>
#include <vector>

// The harness reads private state (RDirNode::spos and friends); nothing else
// changes. Everything that must not see the defines is included first:
// standard headers, GLM, and Core's templates.
#include "core/vectors.h"
#include "core/plane.h"
#include "core/bounds.h"
// Frustum keeps its planes private by default (no access specifier).
#define class struct
#include "core/frustum.h"
#undef class
#include "core/quadtree.h"
#include "spline.h"
#include "gource_settings.h"
#define private public
#define protected public
#include "pawn.h"
// RDirNode members are private by default (no access specifier).
#define class struct
#include "dirnode.h"
#include "file.h"
#undef class
#undef private
#undef protected

uint64_t port_rand_counter = 0;
uint64_t port_rand_seed = 31;

GourceSettings gGourceSettings;
SDLAppDisplay display;
FXFontManager fontmanager;

static RDirNode* root = 0;
static std::map<std::string, RFile*> files;
static Bounds2D dir_bounds;
static QuadTree* dirNodeTree = 0;
static int tag_seq = 1;

static unsigned bits(float f) {
    unsigned u;
    std::memcpy(&u, &f, sizeof u);
    return u;
}

// Gource::addFile
static RFile* addFile(const std::string& filename) {
    std::string file_as_dir = filename;
    if(file_as_dir[file_as_dir.size()-1] != '/') file_as_dir.append("/");
    if(root->isDir(file_as_dir)) return 0;

    RFile* file = new RFile(filename, vec3(1.0f, 1.0f, 1.0f), vec2(0.0, 0.0), tag_seq++);
    files[filename] = file;
    root->addFile(file);
    while(root->getParent() != 0) root = root->getParent();
    file->colourize();
    return file;
}

static void addTouched(const std::string& filename, time_t ts) {
    RFile* f = addFile(filename);
    if(f) f->touch(ts, vec3(0.0f, 1.0f, 0.0f));
}

// Gource::deleteFile (tree part)
static void deleteFile(RFile* file) {
    root->removeFile(file);
    files.erase(file->fullpath);
    delete file;
}

// dir half of Gource::updateBounds
static void updateDirBounds() {
    dir_bounds.reset();
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it != gGourceDirMap.end(); it++) {
        RDirNode* node = it->second;
        if(node->isVisible()) {
            node->updateQuadItemBounds();
            dir_bounds.update(node->quadItemBounds);
        }
    }
}

// Gource::interactDirs
static void interactDirs() {
    Bounds2D quadtreebounds = dir_bounds;
    quadtreebounds.min -= vec2(1.0f, 1.0f);
    quadtreebounds.max += vec2(1.0f, 1.0f);
    if(dirNodeTree != 0) delete dirNodeTree;
    int max_depth = 1;
    if(dir_bounds.area() > 10000.0) max_depth = 6;
    dirNodeTree = new QuadTree(quadtreebounds, max_depth, 1);
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it != gGourceDirMap.end(); it++) {
        RDirNode* node = it->second;
        if(!node->empty()) dirNodeTree->addItem(node);
    }
}

static void step(float dt) {
    for(std::vector<RFile*>::iterator it = gGourceRemovedFiles.begin(); it != gGourceRemovedFiles.end(); it++) {
        deleteFile(*it);
    }
    gGourceRemovedFiles.clear();
    updateDirBounds();
    updateDirBounds();
    interactDirs();
    root->applyForces(*dirNodeTree);
    root->logic(dt);
}

static void snapshot(const char* scenario, int frame) {
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it != gGourceDirMap.end(); it++) {
        RDirNode* n = it->second;
        std::printf("%s\t%d\tdir\t%s\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\n", scenario, frame, it->first.c_str(),
            bits(n->pos.x), bits(n->pos.y), bits(n->spos.x), bits(n->spos.y),
            bits(n->dir_radius), bits(n->col.x));
    }
    for(std::map<std::string,RFile*>::iterator it = files.begin(); it != files.end(); it++) {
        RFile* f = it->second;
        vec2 p = f->getAbsolutePos();
        std::printf("%s\t%d\tfile\t%s\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\n", scenario, frame, it->first.c_str(),
            bits(p.x), bits(p.y), bits(f->pos.x), bits(f->pos.y),
            bits(f->getAlpha()), bits(f->getColour().y));
    }
}

static const char* paths[] = {
    "/src/main.rs",
    "/README.md",
    "/src/lib.rs",
    "/src/core/a.rs",
    "/src/core/b.rs",
    "/src/core/c.rs",
    "/src/render/gl.rs",
    "/src/render/shader.glsl",
    "/docs/index.md",
    "/docs/api/x.md",
    "/tests/t1.rs",
    "/tests/t2.rs",
    "/data/fonts/FreeSans.ttf",
    "/data/file.png",
};

static void run(const char* scenario, float elasticity, float file_idle_time) {
    gGourceSettings.elasticity = elasticity;
    gGourceSettings.file_idle_time = file_idle_time;
    root = new RDirNode(0, "/");
    tag_seq = 1;
    port_rand_counter = 0;

    std::set<int> report = { 0, 1, 2, 5, 10, 30, 60, 100, 101, 150, 151, 200, 201, 250, 299 };

    const int npaths = sizeof(paths) / sizeof(paths[0]);
    for(int i = 0; i < npaths; i++) addTouched(paths[i], i);

    const float dt = 1.0f / 60.0f;
    for(int frame = 0; frame < 300; frame++) {
        if(frame == 100) {
            std::map<std::string, RFile*>::iterator it = files.find("/src/core/b.rs");
            if(it != files.end()) it->second->remove(100);
        }
        if(frame == 150) {
            root->rotate(0.0998334166f, 0.995004165f);
        }
        if(frame == 200) {
            addTouched("/other/deep/x.c", 200);
            addTouched("/src/render/gl/extra.rs", 200);
        }
        step(dt);
        if(report.count(frame)) snapshot(scenario, frame);
    }

    // Gource::reset (tree part)
    for(std::map<std::string,RFile*>::iterator it = files.begin(); it != files.end(); it++) delete it->second;
    files.clear();
    gGourceRemovedFiles.clear();
    delete dirNodeTree;
    dirNodeTree = 0;
    delete root;
    root = 0;
}

// Frustum planes and intersection tests for a few cameras, on the stub
// display (1280x720).
static void frustums() {
    std::printf("# camera\tkind\tindex\tvalues...\n");
    const vec3 cams[][2] = {
        { vec3(0.0f, 0.0f, -300.0f), vec3(0.0f, 0.0f, 0.0f) },
        { vec3(123.5f, -77.25f, -850.0f), vec3(123.5f, -77.25f, 0.0f) },
        { vec3(5.0f, 5.0f, -50.0f), vec3(5.0f, 5.0f, 0.0f) },
        { vec3(10.0f, -20.0f, -400.0f), vec3(0.0f, 0.0f, 0.0f) },
    };
    for(int c = 0; c < 4; c++) {
        Frustum f(cams[c][0], cams[c][1], vec3(0.0f, -1.0f, 0.0f), 90.0f, 0.1f, 10001.0f);
        for(int i = 0; i < 6; i++) {
            std::printf("%d\tplane\t%d\t%08x\t%08x\t%08x\t%08x\n", c, i,
                bits(f.planes[i].normal.x), bits(f.planes[i].normal.y),
                bits(f.planes[i].normal.z), bits(f.planes[i].d));
        }
        int k = 0;
        for(int gx = -12; gx <= 12; gx++) {
            for(int gy = -8; gy <= 8; gy++) {
                vec2 centre = vec2(cams[c][0].x, cams[c][0].y) + vec2(gx * 50.0f, gy * 41.0f);
                float r = 3.0f + (float) ((gx * 7 + gy * 13) & 15);
                Bounds2D b(centre - vec2(r, r), centre + vec2(r, r));
                std::printf("%d\tbox\t%d\t%d\t%d\n", c, k++, f.intersects(b) ? 1 : 0,
                    f.contains(vec3(centre.x, centre.y, 0.0f)) ? 1 : 0);
            }
        }
    }
}

int main(int argc, char** argv) {
    if(argc > 1 && std::string(argv[1]) == "frustum") {
        frustums();
        return 0;
    }

    std::printf("# scenario\tframe\tkind\tpath\tpos.x|abs.x\tpos.y|abs.y\tspos.x|rel.x\tspos.y|rel.y\tradius|alpha\tcol.r|colour.g\n");
    run("plain", 0.0f, 0.0f);
    run("elastic", 0.5f, 0.0f);
    run("idle", 0.0f, 1.5f);
    return 0;
}
