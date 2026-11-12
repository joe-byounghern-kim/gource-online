// Golden-value generator for gource-core's simulation port.
//
// Compiles Gource's own user.cpp, action.cpp, zoomcamera.cpp, dirnode.cpp,
// file.cpp, pawn.cpp and Core's quadtree, frustum, plane, stringhash and
// vectors against the no-op stubs in stubs/ (plus ../dirtree/stubs), and
// drives them with the parts of gource.cpp that run every frame, copied
// below with only the GL, input, caption and file key code removed:
// readLog, processCommit, addFileAction, addFile, addUser, deleteFile,
// deleteUser, updateBounds, interactUsers, updateUsers, interactDirs,
// updateDirs, updateCamera, logic and update. The input is scenario.log
// (custom log format, grouped like custom.cpp). Snapshots of users, the
// camera and the clock (every 20 frames) and of dirs and files (every 100
// frames) are printed as bit patterns; src/sim/tests.rs replays the same log
// through Simulation::update and compares every row.
//
// rand() in dirnode.cpp and user.cpp is replaced by the port's keyed
// streams (port_rand.h), the one intended difference from a stock build.

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <deque>
#include <fstream>
#include <list>
#include <map>
#include <set>
#include <string>
#include <vector>

#include "core/vectors.h"
#include "core/plane.h"
#include "core/bounds.h"
#define class struct
#include "core/frustum.h"
#undef class
#include "core/quadtree.h"
#include "spline.h"
#include "gource_settings.h"
#define private public
#define protected public
#include "pawn.h"
#define class struct
#include "dirnode.h"
#include "file.h"
#include "user.h"
#include "action.h"
#include "zoomcamera.h"
#undef class
#undef private
#undef protected

uint64_t port_rand_seed = 31;
std::map<std::string, uint64_t> port_rand_counters;

GourceSettings gGourceSettings;
SDLAppDisplay display;
FXFontManager fontmanager;
TextureManager texturemanager;

int gGourceUserInnerLoops = 0;
int gGourceMaxQuadTreeDepth = 6;

static unsigned bits(float f) {
    unsigned u;
    std::memcpy(&u, &f, sizeof u);
    return u;
}

// ---------------------------------------------------------------------------
// RCommit (formats/commitlog.cpp) and the custom log reader (custom.cpp)

struct RCommitFile {
    std::string filename;
    std::string action;
    vec3 colour;
    RCommitFile(const std::string& f, const std::string& a, vec3 c) : filename(f), action(a), colour(c) {
        if(filename[0] != '/') filename.insert(0, 1, '/');
    }
};

struct RCommit {
    time_t timestamp = 0;
    std::string username;
    std::list<RCommitFile> files;

    static vec3 fileColour(const std::string& filename) {
        size_t slash = filename.rfind('/');
        size_t dot   = filename.rfind('.');
        if(dot != std::string::npos && dot+1<filename.size() && (slash == std::string::npos || slash < dot)) {
            std::string file_ext = filename.substr(dot+1);
            return colourHash(file_ext);
        } else {
            return vec3(1.0, 1.0, 1.0);
        }
    }
};

static std::vector<RCommit> log_commits;
static size_t log_index = 0;

static void readScenario(const char* path) {
    std::ifstream in(path);
    std::string line;
    RCommit commit;
    bool have = false;
    while(std::getline(in, line)) {
        std::vector<std::string> f;
        size_t start = 0;
        while(true) {
            size_t sep = line.find('|', start);
            f.push_back(line.substr(start, sep == std::string::npos ? std::string::npos : sep - start));
            if(sep == std::string::npos) break;
            start = sep + 1;
        }
        time_t ts = (time_t) atoll(f[0].c_str());
        if(have && (commit.timestamp != ts || commit.username != f[1])) {
            log_commits.push_back(commit);
            commit = RCommit();
            have = false;
        }
        if(!have) {
            commit.timestamp = ts;
            commit.username = f[1];
            have = true;
        }
        vec3 colour;
        if(f.size() >= 5 && f[4].size() == 6) {
            int r, g, b;
            sscanf(f[4].c_str(), "%02x%02x%02x", &r, &g, &b);
            colour = vec3(r, g, b);
            colour /= 255.0f;
        } else {
            colour = RCommit::fileColour(f[3]);
        }
        commit.files.push_back(RCommitFile(f[3], f[2], colour));
    }
    if(have) log_commits.push_back(commit);
}

// ---------------------------------------------------------------------------
// Gource state (gource.h)

static ZoomCamera camera;
static bool manual_zoom = false;
static bool manual_rotate = false;
static bool manual_camera = false;
static float rotation_remaining_angle = 0.0f;
static float rotate_angle = 0.0f;
static bool stop_position_reached = false;
static int tag_seq = 1, commit_seq = 1;
static RFile* hoverFile = 0;
static RFile* selectedFile = 0;
static RUser* hoverUser = 0;
static RUser* selectedUser = 0;
static RDirNode* root = 0;
static float max_tick_rate = 1.0 / 60.0;
static time_t currtime = 0;
static time_t lasttime = 0;
static float runtime = 0.0f;
static float subseconds = 0.0f;
static float idle_time = 0.0f;
static bool track_users = false;
static Bounds2D dir_bounds;
static Bounds2D user_bounds;
static Bounds2D active_user_bounds;
static size_t commitqueue_max_size = 100;
static float starting_z = -300.0f;
static std::deque<RCommit> commitqueue;
static std::map<std::string, RUser*> users;
static std::map<std::string, RFile*> files;
static std::map<int, RUser*> tagusermap;
static QuadTree* dirNodeTree = 0;
static QuadTree* userTree = 0;

static void selectUser(RUser* user) {
    if(user!=0 && selectedUser==user) return;
    if(selectedFile != 0) {
        selectedFile->setSelected(false);
        selectedFile = 0;
    }
    if(selectedUser != 0) {
        selectedUser->setSelected(false);
        selectedUser = 0;
    }
    if(user == 0) {
        camera.lockOn(false);
        return;
    }
    selectedUser = user;
    selectedUser->setSelected(true);
    if(track_users) camera.lockOn(true);
}

static void selectFile(RFile* file) {
    if(file!=0 && selectedFile==file) return;
    if(selectedUser != 0) {
        selectedUser->setSelected(false);
        selectedUser = 0;
    }
    if(selectedFile != 0) {
        selectedFile->setSelected(false);
        selectedFile = 0;
    }
    if(file == 0) return;
    selectedFile = file;
    selectedFile->setSelected(true);
}

static void deleteFile(RFile* file) {
    root->removeFile(file);
    if(hoverFile == file) hoverFile = 0;
    if(selectedFile == file) selectFile(0);
    for(std::map<std::string,RUser*>::iterator it = users.begin(); it!=users.end(); it++) {
        RUser* user = it->second;
        user->fileRemoved(file);
    }
    files.erase(file->fullpath);
    delete file;
}

static RFile* addFile(const RCommitFile& cf) {
    std::string file_as_dir = cf.filename;
    if(file_as_dir[file_as_dir.size()-1] != '/') file_as_dir.append("/");
    if(root->isDir(file_as_dir)) return 0;
    int tagid = tag_seq++;
    RFile* file = new RFile(cf.filename, cf.colour, vec2(0.0,0.0), tagid);
    files[cf.filename] = file;
    root->addFile(file);
    while(root->getParent() != 0) {
        root = root->getParent();
    }
    return file;
}

static RUser* addUser(const std::string& username) {
    vec2 pos;
    if(dir_bounds.area() > 0) {
        pos = dir_bounds.centre();
    } else {
        pos = vec2(0,0);
    }
    int tagid = tag_seq++;
    RUser* user = new RUser(username, pos, tagid);
    users[username]   = user;
    tagusermap[tagid] = user;
    return user;
}

static void deleteUser(RUser* user) {
    if(hoverUser == user) hoverUser = 0;
    if(selectedUser == user) selectUser(0);
    users.erase(user->getName());
    tagusermap.erase(user->getTagID());
    delete user;
}

static void readLog() {
    if(stop_position_reached) return;
    while(log_index < log_commits.size() && (commitqueue.empty() || (commitqueue.back().timestamp <= currtime && commitqueue.size() < commitqueue_max_size)) ) {
        commitqueue.push_back(log_commits[log_index++]);
    }
}

static void addFileAction(const RCommit& commit, const RCommitFile& cf, RFile* file, float t) {
    RUser* user = 0;
    std::map<std::string, RUser*>::iterator seen_user = users.find(commit.username);
    if(seen_user != users.end()) user = seen_user->second;
    if(user == 0) {
        user = addUser(commit.username);
    }
    RAction* userAction = 0;
    commit_seq++;
    if(cf.action == "D") {
        userAction = new RemoveAction(user, file, commit.timestamp, t);
    } else {
        if(cf.action == "A") {
            userAction = new CreateAction(user, file, commit.timestamp, t);
        } else {
            userAction = new ModifyAction(user, file, commit.timestamp, t, cf.colour);
        }
    }
    user->addAction(userAction);
}

static void processCommit(const RCommit& commit, float t) {
    for(std::list<RCommitFile>::const_iterator it = commit.files.begin(); it != commit.files.end(); it++) {
        const RCommitFile& cf = *it;
        RFile* file = 0;
        if(!cf.filename.empty() && cf.filename[cf.filename.size()-1] == '/') {
            if(cf.action != "D") continue;
            std::list<RDirNode*> dirs;
            root->findDirs(cf.filename, dirs);
            for(std::list<RDirNode*>::iterator it = dirs.begin(); it != dirs.end(); it++) {
                RDirNode* dir = (*it);
                std::list<RFile*> dir_files;
                dir->getFilesRecursive(dir_files);
                for(std::list<RFile*>::iterator it = dir_files.begin(); it != dir_files.end(); it++) {
                    RFile* file = *it;
                    addFileAction(commit, cf, file, t);
                }
            }
            continue;
        }
        std::map<std::string, RFile*>::iterator seen_file = files.find(cf.filename);
        if(seen_file != files.end()) file = seen_file->second;
        if(file == 0) {
            file = addFile(cf);
            if(!file) continue;
        }
        addFileAction(commit, cf, file, t);
    }
}

static void interactUsers() {
    Bounds2D quadtreebounds = user_bounds;
    quadtreebounds.min -= vec2(1.0f, 1.0f);
    quadtreebounds.max += vec2(1.0f, 1.0f);
    if(userTree != 0) delete userTree;
    int max_depth = 1;
    if(dir_bounds.area() > 10000.0) {
        max_depth = gGourceMaxQuadTreeDepth;
    }
    userTree = new QuadTree(quadtreebounds, max_depth, 1);
    for(std::map<std::string,RUser*>::iterator it = users.begin(); it!=users.end(); it++) {
        RUser* user = it->second;
        userTree->addItem(user);
    }
    for(std::map<std::string,RUser*>::iterator ait = users.begin(); ait!=users.end(); ait++) {
        RUser* a = ait->second;
        UserForceFunctor uff(a);
        userTree->visitItemsInBounds(a->quadItemBounds, uff);
        gGourceUserInnerLoops += uff.getLoopCount();
        a->applyForceToActions();
    }
}

static void updateBounds() {
    user_bounds.reset();
    active_user_bounds.reset();
    for(std::map<std::string,RUser*>::iterator it = users.begin(); it!=users.end(); it++) {
        RUser* user = it->second;
        user->updateQuadItemBounds();
        user_bounds.update(user->quadItemBounds);
        if(!user->isIdle()) {
            active_user_bounds.update(user->quadItemBounds);
        }
    }
    dir_bounds.reset();
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it!=gGourceDirMap.end(); it++) {
        RDirNode* node = it->second;
        if(node->isVisible()) {
            node->updateQuadItemBounds();
            dir_bounds.update(node->quadItemBounds);
        }
    }
}

static void updateUsers(float t, float dt) {
    std::vector<RUser*> inactiveUsers;
    size_t idle_users = 0;
    for(std::map<std::string,RUser*>::iterator it = users.begin(); it!=users.end(); it++) {
        RUser* u = it->second;
        u->logic(t, dt);
        if(u->isFading() && selectedUser == u) {
            selectUser(0);
        }
        if(u->isInactive()) {
            inactiveUsers.push_back(u);
        }
        if(u->isIdle()) {
            idle_users++;
        }
    }
    if(idle_users==users.size()) {
        idle_time += dt;
    } else {
        idle_time = 0;
    }
    for(std::vector<RUser*>::iterator it = inactiveUsers.begin(); it != inactiveUsers.end(); it++) {
        deleteUser(*it);
    }
}

static void interactDirs() {
    Bounds2D quadtreebounds = dir_bounds;
    quadtreebounds.min -= vec2(1.0f, 1.0f);
    quadtreebounds.max += vec2(1.0f, 1.0f);
    if(dirNodeTree !=0) delete dirNodeTree;
    int max_depth = 1;
    if(dir_bounds.area() > 10000.0) {
        max_depth = gGourceMaxQuadTreeDepth;
    }
    dirNodeTree = new QuadTree(quadtreebounds, max_depth, 1);
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it!=gGourceDirMap.end(); it++) {
        RDirNode* node = it->second;
        if(!node->empty()) {
            dirNodeTree->addItem(node);
        }
    }
}

static void updateDirs(float dt) {
    root->applyForces(*dirNodeTree);
    root->logic(dt);
}

static void updateCamera(float dt) {
    bool auto_rotate = !manual_rotate && !gGourceSettings.disable_auto_rotate;
    if(manual_camera) {
        // no input in the harness
    } else {
        Bounds2D cambounds;
        if(track_users && (selectedFile !=0 || selectedUser !=0)) {
            Bounds2D focusbounds;
            if(selectedUser !=0) focusbounds.update(selectedUser->getPos());
            if(selectedFile !=0) focusbounds.update(selectedFile->getAbsolutePos());
            cambounds = focusbounds;
        } else {
            if(track_users && idle_time==0) cambounds = active_user_bounds;
            else cambounds = dir_bounds;
        }
        camera.adjust(cambounds, !manual_zoom);
    }
    camera.logic(dt);
    if(auto_rotate) {
        if(rotation_remaining_angle > 0.0f) {
            float angle_rate = std::max(dt, (float) (1.0f - fabs((rotation_remaining_angle / 90.0f) - 0.5) * 2.0f)) * dt;
            rotate_angle = std::min(rotation_remaining_angle, 90.0f * angle_rate);
            rotation_remaining_angle -= rotate_angle;
            rotate_angle *= DEGREES_TO_RADIANS;
        } else if(dir_bounds.area() > 10000.0f) {
            float aspect_ratio = display.width / (float) display.height;
            float bounds_ratio = (aspect_ratio > 1.0f) ? dir_bounds.width() / dir_bounds.height() : dir_bounds.height() / dir_bounds.width();
            if(bounds_ratio < 0.67f) {
                rotation_remaining_angle = 90.0f;
            }
        }
    } else {
        rotation_remaining_angle = 0.0f;
    }
}

static void logic(float t, float dt) {
    if(rotate_angle != 0.0f) {
        float s = sinf(rotate_angle);
        float c = cosf(rotate_angle);
        root->rotate(s, c);
        for(std::map<std::string,RUser*>::iterator it = users.begin(); it!=users.end(); it++) {
            RUser* user = it->second;
            vec2 rotated_user_pos = rotate_vec2(user->getPos(), s, c);
            user->setPos(rotated_user_pos);
        }
        rotate_angle = 0.0f;
    }

    if(commitqueue.empty()) {
        readLog();
    }

    if(currtime==0 && !commitqueue.empty()) {
        currtime   = lasttime = commitqueue[0].timestamp;
        subseconds = 0.0;
    }

    float time_inc = (dt * 86400.0 * gGourceSettings.days_per_second);
    int seconds    = (int) time_inc;

    subseconds += time_inc - ((float) seconds);

    if(subseconds >= 1.0) {
        currtime   += (int) subseconds;
        subseconds -= (int) subseconds;
    }

    currtime += seconds;

    for(std::vector<RFile*>::iterator it = gGourceRemovedFiles.begin(); it != gGourceRemovedFiles.end(); it++) {
        deleteFile(*it);
    }
    gGourceRemovedFiles.clear();

    while(!commitqueue.empty()) {
        RCommit commit = commitqueue.front();
        if(gGourceSettings.auto_skip_seconds>=0.0 && idle_time >= gGourceSettings.auto_skip_seconds && !stop_position_reached) {
            currtime = lasttime = commit.timestamp;
            idle_time = 0.0;
        }
        if(commit.timestamp > currtime) break;
        processCommit(commit, t);
        if(gGourceSettings.no_time_travel) {
            if(commit.timestamp > lasttime) {
                lasttime = commit.timestamp;
            }
        } else {
            if(lasttime > commit.timestamp) {
                currtime = commit.timestamp;
            }
            lasttime = commit.timestamp;
        }
        subseconds = 0.0;
        commitqueue.pop_front();
    }

    gGourceUserInnerLoops = 0;
    gGourceDirNodeInnerLoops = 0;
    gGourceFileInnerLoops = 0;

    updateBounds();
    interactUsers();
    updateUsers(t, dt);
    interactDirs();
    updateDirs(dt);
    updateCamera(dt);
}

static void update(float dt) {
    float scaled_dt = std::min(dt, max_tick_rate);
    scaled_dt *= gGourceSettings.time_scale;
    runtime += scaled_dt;
    logic(runtime, scaled_dt);
}

// ---------------------------------------------------------------------------

static void snapshot(int frame) {
    vec3 cp = camera.getPos();
    std::printf("%d\tclock\t%lld\t%lld\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\t%zu\t%zu\n", frame,
        (long long) currtime, (long long) lasttime, bits(subseconds), bits(idle_time),
        bits(cp.x), bits(cp.y), bits(cp.z), bits(rotation_remaining_angle), commitqueue.size(), log_index);
    for(std::map<std::string,RUser*>::iterator it = users.begin(); it != users.end(); it++) {
        RUser* u = it->second;
        std::printf("%d\tuser\t%s\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\t%08x\t%zu\t%zu\t%zu\t%zu\n", frame, it->first.c_str(),
            bits(u->pos.x), bits(u->pos.y), bits(u->accel.x), bits(u->accel.y),
            bits(u->getAlpha()), bits(u->getColour().x), bits(u->action_interval),
            u->actions.size(), u->activeActions.size(), u->actionCount, u->activeCount);
    }
    if(frame % 100 != 0) return;
    for(std::map<std::string,RDirNode*>::iterator it = gGourceDirMap.begin(); it != gGourceDirMap.end(); it++) {
        RDirNode* n = it->second;
        std::printf("%d\tdir\t%s\t%08x\t%08x\t%08x\t%08x\t%08x\n", frame, it->first.c_str(),
            bits(n->pos.x), bits(n->pos.y), bits(n->spos.x), bits(n->dir_radius), bits(n->col.x));
    }
    for(std::map<std::string,RFile*>::iterator it = files.begin(); it != files.end(); it++) {
        RFile* f = it->second;
        vec2 p = f->getAbsolutePos();
        vec3 c = f->getColour();
        std::printf("%d\tfile\t%s\t%08x\t%08x\t%08x\t%08x\t%08x\t%d\n", frame, it->first.c_str(),
            bits(p.x), bits(p.y), bits(f->getAlpha()), bits(c.x), bits(c.z), f->isHidden() ? 1 : 0);
    }
}

int main(int argc, char** argv) {
    gGourceSettings.file_graphic = texturemanager.grab("file.png");
    readScenario(argc > 1 ? argv[1] : "scenario.log");

    camera = ZoomCamera(vec3(0,0, starting_z), vec3(0.0, 0.0, 0.0), gGourceSettings.camera_zoom_default, gGourceSettings.camera_zoom_max);
    camera.setPadding(gGourceSettings.padding);

    root = new RDirNode(0, "/");

    std::printf("# frame\tkind\t...\n");
    const int frames = 2400;
    for(int frame = 1; frame <= frames; frame++) {
        update(1.0f / 60.0f);
        if(frame % 20 == 0) snapshot(frame);
    }
    return 0;
}
