// SPDX-License-Identifier: GPL-3.0-or-later

//! Tree building, reaping, forces and file timing, driven through the
//! [`DirTree`] API the simulation uses.

use glam::{Vec2, Vec3};

use super::*;
use crate::vectors::PI;

fn settings() -> Settings {
    Settings::default()
}

fn add(tree: &mut DirTree, path: &str) -> FileId {
    let s = settings();
    let id = tree.add_file(path, Vec3::ONE, 0, &s).expect("file added");
    tree.files[id].colourize(tree.hash_seed);
    id
}

/// Add and touch (what a CreateAction does when it is applied).
fn add_touched(tree: &mut DirTree, path: &str, ts: i64) -> FileId {
    let id = add(tree, path);
    tree.touch_file(id, ts, Vec3::new(0.0, 1.0, 0.0));
    id
}

/// The dir side of one `Gource::logic` frame.
fn step(tree: &mut DirTree, dt: f32, s: &Settings, scratch: &mut Vec<FileId>) {
    tree.process_removed_files(scratch);
    tree.update_dir_bounds();
    tree.update_dir_bounds();
    tree.interact_dirs();
    tree.update_dirs(dt, s);
}

fn dir_paths(tree: &DirTree) -> Vec<&str> {
    tree.dir_map.keys().map(String::as_str).collect()
}

fn dir(tree: &DirTree, path: &str) -> DirId {
    tree.dir_map[path]
}

fn children(tree: &DirTree, path: &str) -> Vec<String> {
    tree.dirs[dir(tree, path)]
        .children
        .iter()
        .map(|&c| tree.dirs[c].abspath.clone())
        .collect()
}

#[test]
fn first_file_renames_the_empty_root() {
    let mut tree = DirTree::new(&settings());
    let f = add(&mut tree, "/src/main.c");
    assert_eq!(dir_paths(&tree), vec!["/src/"]);
    assert_eq!(tree.dirs[tree.root()].abspath, "/src/");
    assert_eq!(tree.files[f].dir, Some(tree.root()));
    assert_eq!(tree.dirs[tree.root()].depth, 1);
    // The new file is hidden until touched: no visible count yet.
    assert_eq!(tree.dirs[tree.root()].visible_count, 0);
    tree.touch_file(f, 1, Vec3::ONE);
    assert_eq!(tree.dirs[tree.root()].visible_count, 1);
    assert!(tree.dirs[tree.root()].visible);
}

#[test]
fn file_outside_root_forks_a_new_root() {
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/src/main.c");
    let old_root = tree.root();
    add(&mut tree, "/README");

    let root = tree.root();
    assert_ne!(root, old_root);
    assert_eq!(tree.dirs[root].abspath, "/");
    assert_eq!(tree.dirs[old_root].parent, Some(root));
    assert_eq!(dir_paths(&tree), vec!["/", "/src/"]);
    assert_eq!(tree.dirs[old_root].depth, 2);
    assert_eq!(tree.dirs[old_root].path_token, "src");
    assert_eq!(tree.dirs[root].files.len(), 1);

    // A deeper fork: a root at /a/b/ meeting /a/c/x forks /a/.
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/a/b/x");
    add(&mut tree, "/a/c/y");
    assert_eq!(tree.dirs[tree.root()].abspath, "/a/");
    assert_eq!(children(&tree, "/a/"), vec!["/a/b/", "/a/c/"]);
}

#[test]
fn renamed_root_keeps_its_old_token_offset() {
    // Root "/" is renamed to "/src/" without re-running adjustPath, so its
    // children compute tokens from offset 1 (Gource quirk).
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/src/main.c");
    add(&mut tree, "/src/a/x.c");
    assert_eq!(tree.dirs[dir(&tree, "/src/a/")].path_token, "src/a");
}

#[test]
fn common_prefix_splits_into_an_intermediate_node() {
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/README");
    add(&mut tree, "/a/b/x.c");
    assert_eq!(children(&tree, "/"), vec!["/a/b/"]);
    let b_pos = tree.dirs[dir(&tree, "/a/b/")].pos;

    add(&mut tree, "/a/c/y.c");
    assert_eq!(dir_paths(&tree), vec!["/", "/a/", "/a/b/", "/a/c/"]);
    assert_eq!(children(&tree, "/"), vec!["/a/"]);
    assert_eq!(children(&tree, "/a/"), vec!["/a/b/", "/a/c/"]);
    // The intermediate node starts at the position of the child it came from.
    assert_eq!(tree.dirs[dir(&tree, "/a/")].pos, b_pos);
    assert_eq!(tree.dirs[dir(&tree, "/a/c/")].depth, 3);
    assert_eq!(tree.dirs[dir(&tree, "/a/c/")].path_token, "c");
    assert_eq!(tree.dirs[dir(&tree, "/a/")].path_token, "a");
}

#[test]
fn empty_dirs_are_reaped_when_files_are_deleted() {
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/README");
    let x = add(&mut tree, "/a/b/x.c");
    let y = add(&mut tree, "/a/c/y.c");

    assert!(tree.delete_file(y).is_some());
    assert_eq!(dir_paths(&tree), vec!["/", "/a/", "/a/b/"]);
    assert!(tree.files.get(y).is_none());
    assert_eq!(tree.find_file("/a/c/y.c"), None);

    tree.delete_file(x);
    assert_eq!(dir_paths(&tree), vec!["/"]);
    assert_eq!(tree.dirs.len(), 1);
    assert_eq!(tree.file_count(), 1);
    // Deleting twice is a no-op.
    assert!(tree.delete_file(x).is_none());
}

#[test]
fn existing_directory_path_is_not_added_as_a_file() {
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/README");
    add(&mut tree, "/a/b/x.c");
    let s = settings();
    assert!(tree.add_file("/a/b", Vec3::ONE, 0, &s).is_none());
    assert!(tree.add_file("/a", Vec3::ONE, 0, &s).is_none());
    assert!(tree.add_file("relative/path", Vec3::ONE, 0, &s).is_none());
    assert!(tree.add_file("/a/bc", Vec3::ONE, 0, &s).is_some());
}

#[test]
fn max_files_limits_additions() {
    let mut tree = DirTree::new(&settings());
    let s = Settings {
        max_files: 2,
        ..settings()
    };
    assert!(tree.add_file("/a", Vec3::ONE, 0, &s).is_some());
    assert!(tree.add_file("/b", Vec3::ONE, 0, &s).is_some());
    assert!(tree.add_file("/c", Vec3::ONE, 0, &s).is_none());
}

#[test]
fn file_turning_into_a_directory_is_force_removed() {
    // tests/logs/file-to-dir.log
    let s = settings();
    let mut scratch = Vec::new();
    let mut tree = DirTree::new(&s);

    let not1 = add_touched(&mut tree, "/path/to/file/Not", 1277787456);
    for _ in 0..10 {
        step(&mut tree, 0.1, &s, &mut scratch);
    }

    let not2 = add_touched(&mut tree, "/path/to/file/Not/Not", 1277787457);
    assert!(tree.files[not1].forced_removal);
    assert!(tree.files[not1].fade_start > 0.0);
    // A forced removal ignores later touches.
    tree.touch_file(not1, 1277787458, Vec3::ONE);
    assert!(tree.files[not1].fade_start > 0.0);

    let not3 = add_touched(&mut tree, "/path/to/file/Not/Not/Not", 1277787457);
    // not2 was only just touched (elapsed 0): fade_start == 0 never expires,
    // exactly like Gource.
    assert!(tree.files[not2].forced_removal);

    for _ in 0..20 {
        step(&mut tree, 0.1, &s, &mut scratch);
    }
    let file = add_touched(&mut tree, "/path/to/file/Not/Not/Not/File", 1277787459);
    assert!(tree.files[not3].forced_removal);
    for _ in 0..20 {
        step(&mut tree, 0.1, &s, &mut scratch);
    }

    assert!(tree.files.get(not1).is_none(), "first file expired");
    assert!(tree.files.get(not3).is_none(), "third file expired");
    assert!(tree.files.contains(not2), "zero fade_start never expires");
    assert!(tree.files.contains(file));
    assert_eq!(
        dir_paths(&tree),
        vec![
            "/path/to/file/",
            "/path/to/file/Not/",
            "/path/to/file/Not/Not/",
            "/path/to/file/Not/Not/Not/",
        ]
    );
    assert_eq!(
        tree.dirs[tree.files[file].dir.unwrap()].abspath,
        "/path/to/file/Not/Not/Not/"
    );
}

#[test]
fn directory_delete_collects_files_recursively() {
    // tests/logs/custom-dir-delete.log
    let mut tree = DirTree::new(&settings());
    let one = add(&mut tree, "/path/to/1.txt");
    let b1 = add(&mut tree, "/path/to/blah/1.txt");
    let b2 = add(&mut tree, "/path/to/blah/2.txt");
    let b3 = add(&mut tree, "/path/to/blah/3.txt");

    let mut dirs = Vec::new();
    tree.find_dirs(tree.root(), "/path/to/", &mut dirs);
    assert_eq!(dirs, vec![tree.root()]);

    let mut files = Vec::new();
    tree.get_files_recursive(dirs[0], &mut files);
    // Each node's files are inserted at the front of the list.
    assert_eq!(files, vec![b1, b2, b3, one]);

    // findDirs returns the nodes closest to the root under a missing prefix.
    let mut tree = DirTree::new(&settings());
    add(&mut tree, "/README");
    add(&mut tree, "/foo/bar1/a");
    add(&mut tree, "/foo/bar2/b");
    add(&mut tree, "/other/c");
    let mut dirs = Vec::new();
    tree.find_dirs(tree.root(), "/foo/", &mut dirs);
    let names: Vec<&str> = dirs
        .iter()
        .map(|&d| tree.dirs[d].abspath.as_str())
        .collect();
    assert_eq!(names, vec!["/foo/"]);
    dirs.clear();
    tree.find_dirs(tree.root(), "/foo/bar1", &mut dirs);
    let names: Vec<&str> = dirs
        .iter()
        .map(|&d| tree.dirs[d].abspath.as_str())
        .collect();
    assert_eq!(names, vec!["/foo/bar1/"]);
}

#[test]
fn files_are_placed_on_rings() {
    let mut tree = DirTree::new(&settings());
    let ids: Vec<FileId> = (0..8)
        .map(|i| add_touched(&mut tree, &format!("/f{i}"), 1))
        .collect();
    let d = tree.root();
    assert_eq!(tree.dirs[d].visible_count, 8);

    let dist: Vec<f32> = ids.iter().map(|&f| tree.files[f].distance).collect();
    // Centre holds 1, ring 1 holds floor(2 * PI) = 6, the last one goes on
    // ring 2 alone (max_files clamped to files_left).
    assert_eq!(dist, vec![0.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 16.0]);
    assert_eq!(tree.files[ids[1]].dest, calc_dest(6, 0));
    assert_eq!(tree.files[ids[6]].dest, calc_dest(6, 5));
    assert_eq!(tree.files[ids[7]].dest, calc_dest(1, 0));

    fn calc_dest(n: i32, i: i32) -> Vec2 {
        crate::dirnode::calc_file_dest(n, i)
    }

    // dir_radius = max(1, sqrt(8 * 16 * PI)) * 1.5
    let file_area = (16.0f64 * PI) as f32;
    let expected = (file_area * 8.0).sqrt() * 1.5;
    assert_eq!(tree.dirs[d].dir_radius, expected);
}

fn build_layout_tree(s: &Settings) -> DirTree {
    let mut tree = DirTree::new(s);
    populate(&mut tree);
    tree
}

fn populate(tree: &mut DirTree) {
    let paths = [
        "/README.md",
        "/src/main.rs",
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
    ];
    for (i, p) in paths.iter().enumerate() {
        add_touched(tree, p, i as i64);
    }
}

fn snapshot(tree: &DirTree) -> Vec<(String, [u32; 2], [u32; 2], u32)> {
    tree.dir_map
        .iter()
        .map(|(p, &d)| {
            let n = &tree.dirs[d];
            (
                p.clone(),
                [n.pos.x.to_bits(), n.pos.y.to_bits()],
                [n.spos.x.to_bits(), n.spos.y.to_bits()],
                n.dir_radius.to_bits(),
            )
        })
        .chain(tree.file_map.iter().map(|(p, &f)| {
            let pos = tree.file_abs_pos(f);
            (
                p.clone(),
                [pos.x.to_bits(), pos.y.to_bits()],
                [0, 0],
                tree.files[f].alpha().to_bits(),
            )
        }))
        .collect()
}

#[test]
fn forces_and_movement_are_deterministic() {
    let s = Settings {
        elasticity: 0.5,
        ..settings()
    };
    let run = || {
        let mut tree = build_layout_tree(&s);
        let mut scratch = Vec::new();
        for frame in 0..300 {
            if frame == 100 {
                let f = tree.find_file("/src/core/b.rs").unwrap();
                tree.remove_file_at(f, 100);
            }
            if frame == 150 {
                tree.rotate(libm::sinf(0.1), libm::cosf(0.1));
            }
            step(&mut tree, 1.0 / 60.0, &s, &mut scratch);
        }
        tree
    };
    let a = run();
    let b = run();
    assert_eq!(snapshot(&a), snapshot(&b));

    // The layout spread out and stayed finite.
    for (_, n) in a.dirs.iter() {
        assert!(n.pos.is_finite() && n.spos.is_finite(), "{}", n.abspath);
    }
    let src = a.dirs[dir(&a, "/src/")].pos;
    let docs = a.dirs[dir(&a, "/docs/")].pos;
    assert!((src - docs).length() > 1.0);
    // The removed file faded out and was deleted.
    assert_eq!(a.find_file("/src/core/b.rs"), None);
    assert!(a.dir_inner_loops > 0);
}

#[test]
fn exact_overlap_uses_the_keyed_rng() {
    let s = settings();
    let mut tree = DirTree::new(&s);
    add_touched(&mut tree, "/README", 0);
    add_touched(&mut tree, "/a/x", 0);
    add_touched(&mut tree, "/b/y", 0);
    let a = dir(&tree, "/a/");
    let b = dir(&tree, "/b/");
    tree.dirs[a].pos = Vec2::new(3.0, 4.0);
    tree.dirs[b].pos = Vec2::new(3.0, 4.0);
    tree.apply_force_dir(a, b);
    let push = tree.dirs[a].accel;
    assert!(
        (push.length() - 1.0).abs() < 1e-6,
        "unit push, got {push:?}"
    );

    // Same seed, same stream: same push after a reset.
    let mut again = DirTree::new(&s);
    add_touched(&mut again, "/README", 0);
    add_touched(&mut again, "/a/x", 0);
    add_touched(&mut again, "/b/y", 0);
    let (a2, b2) = (dir(&again, "/a/"), dir(&again, "/b/"));
    again.dirs[a2].pos = Vec2::new(3.0, 4.0);
    again.dirs[b2].pos = Vec2::new(3.0, 4.0);
    again.apply_force_dir(a2, b2);
    assert_eq!(again.dirs[a2].accel, push);
}

#[test]
fn idle_files_fade_expire_and_are_deleted() {
    let s = Settings {
        file_idle_time: 1.0,
        ..settings()
    };
    let mut scratch = Vec::new();
    let mut tree = DirTree::new(&s);
    add_touched(&mut tree, "/README", 0);
    let x = add_touched(&mut tree, "/a/x.c", 0);

    let dt = 0.25;
    let mut expired_frame = None;
    for frame in 1..=12 {
        step(&mut tree, dt, &s, &mut scratch);
        if expired_frame.is_none() && tree.removed_files.contains(&x) {
            expired_frame = Some(frame);
        }
    }
    // elapsed > 1.0 at frame 5 starts the fade; it completes at frame 9 and
    // the file is deleted (and /a/ reaped) at the start of frame 10.
    assert_eq!(expired_frame, Some(9));
    assert!(tree.files.get(x).is_none());
    assert_eq!(tree.find_file("/README"), None, "README idled out too");
    assert_eq!(dir_paths(&tree), vec!["/"]);
}

#[test]
fn touching_an_expired_file_revives_it() {
    let s = Settings {
        file_idle_time: 0.5,
        ..settings()
    };
    let mut tree = DirTree::new(&s);
    let x = add_touched(&mut tree, "/x.c", 0);
    for _ in 0..8 {
        tree.update_dir_bounds();
        tree.interact_dirs();
        tree.update_dirs(0.25, &s);
    }
    assert!(tree.files[x].expired);
    assert_eq!(tree.removed_files, vec![x]);
    tree.touch_file(x, 5, Vec3::ONE);
    assert!(!tree.files[x].expired);
    assert!(tree.removed_files.is_empty());
    assert_eq!(tree.files[x].fade_start, -1.0);
}

#[test]
fn an_expired_file_is_queued_once_even_after_reviving() {
    let s = Settings {
        file_idle_time: 0.5,
        ..settings()
    };
    let mut tree = DirTree::new(&s);
    let x = add_touched(&mut tree, "/x.c", 0);
    let step = |tree: &mut DirTree| {
        tree.update_dir_bounds();
        tree.interact_dirs();
        tree.update_dirs(0.25, &s);
    };
    for _ in 0..8 {
        step(&mut tree);
    }
    assert_eq!(tree.removed_files, vec![x]);
    tree.touch_file(x, 5, Vec3::ONE);
    // expires again; the queue never holds it twice
    for _ in 0..40 {
        step(&mut tree);
    }
    assert_eq!(tree.removed_files, vec![x]);
}

#[test]
fn removal_is_cancelled_only_by_later_touches() {
    let s = settings();
    let mut tree = DirTree::new(&s);
    let x = add_touched(&mut tree, "/x.c", 10);
    tree.update_dirs(0.5, &s);
    tree.remove_file_at(x, 20);
    tree.touch_file(x, 15, Vec3::ONE);
    assert!(tree.files[x].removing, "older touch ignored");
    tree.touch_file(x, 25, Vec3::ONE);
    assert!(!tree.files[x].removing);
}

#[test]
fn reset_starts_from_an_empty_root() {
    let s = settings();
    let mut scratch = Vec::new();
    let mut tree = build_layout_tree(&s);
    for _ in 0..30 {
        step(&mut tree, 1.0 / 60.0, &s, &mut scratch);
    }
    let old_root = tree.root();
    tree.reset(&s);
    assert_eq!(dir_paths(&tree), vec!["/"]);
    assert_eq!(tree.file_count(), 0);
    assert!(tree.removed_files.is_empty());
    assert!(tree.dirs.get(old_root).is_none());

    // Replaying after a reset gives the same layout as a fresh tree.
    populate(&mut tree);
    let mut fresh = build_layout_tree(&s);
    for _ in 0..30 {
        step(&mut fresh, 1.0 / 60.0, &s, &mut scratch);
        step(&mut tree, 1.0 / 60.0, &s, &mut scratch);
    }
    assert_eq!(snapshot(&fresh), snapshot(&tree));
}

#[test]
fn rotation_and_frustum() {
    let s = settings();
    let mut tree = build_layout_tree(&s);
    let mut scratch = Vec::new();
    for _ in 0..60 {
        step(&mut tree, 1.0 / 60.0, &s, &mut scratch);
    }
    let d = dir(&tree, "/src/");
    let before = tree.dirs[d].pos;
    tree.rotate(1.0, 0.0);
    assert_eq!(tree.dirs[d].pos, Vec2::new(-before.y, before.x));
    tree.rotate_about(-1.0, 0.0, Vec2::ZERO);
    assert_eq!(tree.dirs[d].pos, before);

    tree.update_dir_bounds();
    let centre = tree.dir_bounds().centre();
    let frustum = Frustum::new(
        Vec3::new(centre.x, centre.y, -300.0),
        Vec3::new(centre.x, centre.y, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        90.0,
        0.1,
        10001.0,
        16.0 / 9.0,
    );
    tree.check_frustum(&frustum);
    assert!(tree.dirs[d].in_frustum);
    let far = Frustum::new(
        Vec3::new(1.0e5, 0.0, -300.0),
        Vec3::new(1.0e5, 0.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        90.0,
        0.1,
        10001.0,
        16.0 / 9.0,
    );
    tree.check_frustum(&far);
    assert!(!tree.dirs[d].in_frustum);
}

#[test]
fn mouse_picking_finds_files_through_the_quadtree() {
    let s = settings();
    let mut tree = build_layout_tree(&s);
    let mut scratch = Vec::new();
    for _ in 0..120 {
        step(&mut tree, 1.0 / 60.0, &s, &mut scratch);
    }
    tree.update_dir_bounds();
    tree.interact_dirs();
    let f = tree.find_file("/src/core/a.rs").unwrap();
    let pos = tree.file_abs_pos(f);
    let mut dirs = Vec::new();
    tree.dir_quadtree().collect_items_at(&mut dirs, pos);
    let hit = dirs.iter().any(|&d| {
        tree.dirs[d]
            .files
            .iter()
            .any(|&x| !tree.files[x].is_hidden() && tree.file_overlaps(x, pos) && x == f)
    });
    assert!(hit);
}

// --- C++ golden layout (tests/golden/dirtree) ---

const GOLDEN_LAYOUT: &str = include_str!("../tests/golden/dirtree_golden.tsv");

const GOLDEN_PATHS: [&str; 14] = [
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
];

const GOLDEN_FRAMES: [i32; 15] = [
    0, 1, 2, 5, 10, 30, 60, 100, 101, 150, 151, 200, 201, 250, 299,
];

fn golden_snapshot(tree: &DirTree, scenario: &str, frame: i32, out: &mut Vec<String>) {
    for (path, &d) in &tree.dir_map {
        let n = &tree.dirs[d];
        out.push(format!(
            "{scenario}\t{frame}\tdir\t{path}\t{:08x}\t{:08x}\t{:08x}\t{:08x}\t{:08x}\t{:08x}",
            n.pos.x.to_bits(),
            n.pos.y.to_bits(),
            n.spos.x.to_bits(),
            n.spos.y.to_bits(),
            n.dir_radius.to_bits(),
            n.col.x.to_bits(),
        ));
    }
    for (path, &f) in &tree.file_map {
        let file = &tree.files[f];
        let p = tree.file_abs_pos(f);
        out.push(format!(
            "{scenario}\t{frame}\tfile\t{path}\t{:08x}\t{:08x}\t{:08x}\t{:08x}\t{:08x}\t{:08x}",
            p.x.to_bits(),
            p.y.to_bits(),
            file.pawn.pos.x.to_bits(),
            file.pawn.pos.y.to_bits(),
            file.alpha().to_bits(),
            file.colour().y.to_bits(),
        ));
    }
}

fn golden_run(scenario: &str, elasticity: f32, file_idle_time: f32, out: &mut Vec<String>) {
    let s = Settings {
        elasticity,
        file_idle_time,
        ..settings()
    };
    let mut tree = DirTree::new(&s);
    let mut scratch = Vec::new();
    for (i, p) in GOLDEN_PATHS.iter().enumerate() {
        add_touched(&mut tree, p, i as i64);
    }
    let dt = 1.0f32 / 60.0;
    for frame in 0..300 {
        if frame == 100
            && let Some(f) = tree.find_file("/src/core/b.rs")
        {
            tree.remove_file_at(f, 100);
        }
        if frame == 150 {
            // Same literals as the C++ harness (sin/cos of 0.1).
            #[allow(clippy::excessive_precision)]
            tree.rotate(0.0998334166, 0.995004165);
        }
        if frame == 200 {
            add_touched(&mut tree, "/other/deep/x.c", 200);
            add_touched(&mut tree, "/src/render/gl/extra.rs", 200);
        }
        step(&mut tree, dt, &s, &mut scratch);
        if GOLDEN_FRAMES.contains(&frame) {
            golden_snapshot(&tree, scenario, frame, out);
        }
    }
}

#[test]
fn layout_matches_cpp_golden() {
    let mut ours = Vec::new();
    golden_run("plain", 0.0, 0.0, &mut ours);
    golden_run("elastic", 0.5, 0.0, &mut ours);
    golden_run("idle", 0.0, 1.5, &mut ours);

    let theirs: Vec<&str> = GOLDEN_LAYOUT
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();

    for (i, (a, b)) in ours.iter().zip(theirs.iter()).enumerate() {
        assert_eq!(a, b, "first difference at golden row {i}");
    }
    assert_eq!(ours.len(), theirs.len(), "row count");
}

/// The per-frame numbering answers every ancestor test as the parent-chain
/// walk does: a deep chain, split nodes and names that are string prefixes
/// of a sibling's (`/a/` beside `/ab/`).
#[test]
fn numbered_ancestor_test_matches_the_walk() {
    let s = settings();
    let mut tree = DirTree::new(&s);
    let mut deep = String::from("/deep/");
    for i in 0..40 {
        deep.push_str(&format!("{i}/"));
        add_touched(&mut tree, &format!("{deep}f.c"), 1);
    }
    for path in [
        "/a/b/x.c",
        "/a/y.c",
        "/a/b/c/z.c",
        "/ab/q.c",
        "/src/m.c",
        "/src/n/o.c",
    ] {
        add_touched(&mut tree, path, 1);
    }

    tree.number_tree();
    assert!(tree.tour_valid);
    let ids: Vec<DirId> = tree.dir_map.values().copied().collect();
    assert!(ids.len() > 45);
    for &a in &ids {
        for &b in &ids {
            assert_eq!(
                tree.is_parent_numbered(a, b),
                tree.is_parent(a, b),
                "{} / {}",
                tree.dirs[a].abspath,
                tree.dirs[b].abspath
            );
        }
    }

    // A child list that disagrees with the parent links drops the
    // numbering, and the walk answers.
    let root = tree.root();
    let stray = tree.dirs[root].children[0];
    let other = tree.dirs[root].children[1];
    tree.dirs[other].children.push(stray);
    tree.number_tree();
    assert!(!tree.tour_valid);
    for &a in &ids {
        assert_eq!(tree.is_parent_numbered(a, root), tree.is_parent(a, root));
    }
}

/// `--elasticity` far beyond the stable range (a crafted URL) and at the
/// fastest time scale keeps every directory finite and near the root.
#[test]
fn extreme_elasticity_keeps_the_layout_finite() {
    for (elasticity, dt) in [(20.0, 1.0 / 60.0), (5.0, 4.0 / 60.0), (1.0e6, 4.0 / 60.0)] {
        let mut s = settings();
        s.elasticity = elasticity;
        let mut tree = build_layout_tree(&s);
        let mut scratch = Vec::new();
        for _ in 0..1200 {
            step(&mut tree, dt, &s, &mut scratch);
        }
        for (path, &d) in &tree.dir_map {
            let p = tree.dirs[d].pos;
            assert!(
                p.is_finite() && p.length() < 1.0e4,
                "{elasticity} {path}: {p}"
            );
        }
    }
}
