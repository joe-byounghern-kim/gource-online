// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Core's `QuadTree` / `QuadNode` (`core/quadtree.cpp`).
//!
//! Rules kept from the C++:
//! - the root has depth 1; a node accepts an item while it has no children and
//!   either its depth is `>= max_node_depth` or it holds fewer than
//!   `max_node_items` items;
//! - a full node splits into four children (top left, top right, bottom left,
//!   bottom right, built from `centre() - min` exactly as the C++ does), moves
//!   its items into every child whose bounds overlap the item, then adds the
//!   new item the same way; an item that overlaps no child is dropped;
//! - `node_count` of an item is the number of leaves holding it, so items
//!   spanning several leaves are visited once per leaf (callers dedupe with
//!   `node_count != 1`, like `DirForceFunctor`);
//! - visiting walks leaves in child order 0..4, skipping empty children and
//!   children whose bounds fail the test; items come out in insertion order
//!   within a leaf.
//!
//! Gource rebuilds its trees from scratch every frame (`new QuadTree`); here
//! [`QuadTree::reset`] keeps the node pool and item buffers, so steady-state
//! rebuilds do not allocate. A node's item list (`std::list` in the C++) is a
//! singly linked list threaded through one shared entry pool, so how items
//! spread over the reused node slots never makes a per-node buffer grow.
//!
//! Items are `T: Copy` handles (entity ids) stored with their bounds. The
//! `std::set<QuadItem*>` results of `getItemsAt` / `getItemsInBounds` are
//! ordered by pointer in the C++, which is not reproducible; the
//! `collect_*` helpers return items sorted by `T`'s `Ord` (entity id order)
//! and deduplicated instead.

use glam::Vec2;

use crate::bounds::Bounds2D;
use crate::frustum::Frustum;

/// One item as stored by the tree.
#[derive(Copy, Clone, Debug)]
pub(crate) struct QuadItem<T> {
    pub(crate) id: T,
    pub(crate) bounds: Bounds2D,
    /// `QuadItem::node_count`: number of leaves holding this item.
    pub(crate) node_count: i32,
}

#[derive(Clone, Debug, Default)]
struct QuadNode {
    bounds: Bounds2D,
    depth: i32,
    /// Index of the first of four consecutive children; 0 = no children
    /// (the root is node 0 and never anyone's child).
    first_child: u32,
    /// First and last entry of the node's item list in `QuadTree::links`
    /// ([`NIL`] when empty), in insertion order (`std::list`).
    head: u32,
    tail: u32,
    len: u32,
}

/// End of a node's item list.
const NIL: u32 = u32::MAX;

/// One entry of a node's item list: an index into `QuadTree::items`.
#[derive(Copy, Clone, Debug)]
struct Link {
    item: u32,
    next: u32,
}

impl QuadNode {
    fn has_children(&self) -> bool {
        self.first_child != 0
    }

    fn has_items(&self) -> bool {
        self.head != NIL
    }

    fn clear_items(&mut self) {
        self.head = NIL;
        self.tail = NIL;
        self.len = 0;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct QuadTree<T> {
    /// Node pool; only `nodes[..node_len]` are live.
    nodes: Vec<QuadNode>,
    node_len: usize,
    items: Vec<QuadItem<T>>,
    /// Entry pool of every node's item list.
    links: Vec<Link>,
    pub(crate) unique_item_count: i32,
    pub(crate) item_count: i32,
    pub(crate) node_count: i32,
    pub(crate) max_node_depth: i32,
    pub(crate) max_node_items: i32,
}

impl<T: Copy> Default for QuadTree<T> {
    fn default() -> Self {
        QuadTree {
            nodes: Vec::new(),
            node_len: 0,
            items: Vec::new(),
            links: Vec::new(),
            unique_item_count: 0,
            item_count: 0,
            node_count: 0,
            max_node_depth: 0,
            max_node_items: 0,
        }
    }
}

impl<T: Copy> QuadTree<T> {
    /// `QuadTree(bounds, max_node_depth, max_node_items)`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn new(bounds: Bounds2D, max_node_depth: i32, max_node_items: i32) -> Self {
        let mut tree = QuadTree::default();
        tree.reset(bounds, max_node_depth, max_node_items);
        tree
    }

    /// Equivalent of deleting the tree and constructing a new one, reusing
    /// the storage.
    pub(crate) fn reset(&mut self, bounds: Bounds2D, max_node_depth: i32, max_node_items: i32) {
        self.node_len = 0;
        self.items.clear();
        self.links.clear();
        self.item_count = 0;
        self.node_count = 0;
        self.unique_item_count = 0;
        self.max_node_depth = max_node_depth;
        self.max_node_items = max_node_items;
        self.alloc_node(bounds, 0);
    }

    /// Drop every node and item (no root), as after `delete tree; tree = 0`.
    pub(crate) fn clear(&mut self) {
        self.node_len = 0;
        self.items.clear();
        self.links.clear();
        self.item_count = 0;
        self.node_count = 0;
        self.unique_item_count = 0;
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn items(&self) -> &[QuadItem<T>] {
        &self.items
    }

    /// `new QuadNode(tree, parent, bounds, parent_depth)`.
    fn alloc_node(&mut self, bounds: Bounds2D, parent_depth: i32) -> usize {
        let index = self.node_len;
        if index < self.nodes.len() {
            let node = &mut self.nodes[index];
            node.bounds = bounds;
            node.depth = parent_depth + 1;
            node.first_child = 0;
            node.clear_items();
        } else {
            self.nodes.push(QuadNode {
                bounds,
                depth: parent_depth + 1,
                first_child: 0,
                head: NIL,
                tail: NIL,
                len: 0,
            });
        }
        self.node_len += 1;
        self.node_count += 1;
        index
    }

    /// `QuadTree::addItem`.
    pub(crate) fn add_item(&mut self, id: T, bounds: Bounds2D) {
        if self.node_len == 0 {
            return;
        }
        let item = self.items.len() as u32;
        self.items.push(QuadItem {
            id,
            bounds,
            node_count: 0,
        });
        self.node_add_item(0, item);
        self.unique_item_count += 1;
    }

    fn allow_more_items(&self, node: usize) -> bool {
        let n = &self.nodes[node];
        // `items.size() < tree->max_node_items` compares as size_t.
        !n.has_children()
            && (n.depth >= self.max_node_depth
                || (n.len as u64) < (self.max_node_items as i64 as u64))
    }

    /// `QuadNode::addItem`.
    fn node_add_item(&mut self, node: usize, item: u32) {
        if self.allow_more_items(node) {
            self.item_count += 1;
            self.items[item as usize].node_count += 1;
            self.push_node_item(node, item);
            return;
        }

        if self.nodes[node].has_children() {
            self.add_to_child(node, item);
            return;
        }

        let bounds = self.nodes[node].bounds;
        let depth = self.nodes[node].depth;

        let average = bounds.centre();
        let middle = average - bounds.min;
        let relmax = bounds.max - bounds.min;

        let quads = [
            // top left
            Bounds2D::new(bounds.min + Vec2::new(0.0, 0.0), bounds.min + middle),
            // top right
            Bounds2D::new(
                bounds.min + Vec2::new(middle.x, 0.0),
                bounds.min + Vec2::new(relmax.x, middle.y),
            ),
            // bottom left
            Bounds2D::new(
                bounds.min + Vec2::new(0.0, middle.y),
                bounds.min + Vec2::new(middle.x, relmax.y),
            ),
            // bottom right
            Bounds2D::new(bounds.min + middle, bounds.max),
        ];

        let first = self.alloc_node(quads[0], depth);
        for q in &quads[1..] {
            self.alloc_node(*q, depth);
        }
        self.nodes[node].first_child = first as u32;

        // Move the node's items down. They stay in the node's list until the
        // loop is done, as in the C++; add_to_child only touches children.
        let mut entry = self.nodes[node].head;
        while entry != NIL {
            let Link { item: oi, next } = self.links[entry as usize];
            self.item_count -= 1;
            self.items[oi as usize].node_count -= 1;
            self.add_to_child(node, oi);
            entry = next;
        }
        self.nodes[node].clear_items();

        self.add_to_child(node, item);
    }

    /// Append `item` to `node`'s list.
    fn push_node_item(&mut self, node: usize, item: u32) {
        let entry = self.links.len() as u32;
        self.links.push(Link { item, next: NIL });
        let n = &mut self.nodes[node];
        if n.tail == NIL {
            n.head = entry;
        } else {
            self.links[n.tail as usize].next = entry;
        }
        n.tail = entry;
        n.len += 1;
    }

    /// `visit(id, node_count)` for every item of `node`, in list order.
    fn visit_node_items<F: FnMut(T, i32)>(&self, node: usize, visit: &mut F) {
        let mut entry = self.nodes[node].head;
        while entry != NIL {
            let link = self.links[entry as usize];
            let item = &self.items[link.item as usize];
            visit(item.id, item.node_count);
            entry = link.next;
        }
    }

    /// `QuadNode::addToChild`.
    fn add_to_child(&mut self, node: usize, item: u32) {
        let first = self.nodes[node].first_child as usize;
        if first == 0 {
            return;
        }
        for i in 0..4 {
            let item_bounds = self.items[item as usize].bounds;
            if self.nodes[first + i].bounds.overlaps(&item_bounds) {
                self.node_add_item(first + i, item);
            }
        }
    }

    fn node_empty(&self, node: usize) -> bool {
        let n = &self.nodes[node];
        !n.has_items() && !n.has_children()
    }

    /// `getChildIndex`: the first child whose bounds contain `pos`.
    fn child_index(&self, node: usize, pos: Vec2) -> Option<usize> {
        let first = self.nodes[node].first_child as usize;
        if first == 0 {
            return None;
        }
        (0..4)
            .map(|i| first + i)
            .find(|&c| self.nodes[c].bounds.contains(pos))
    }

    /// `visitItemsInBounds(bounds, visit)`; `visit(id, node_count)`.
    pub(crate) fn visit_items_in_bounds<F: FnMut(T, i32)>(&self, bounds: &Bounds2D, visit: &mut F) {
        if self.node_len > 0 {
            self.visit_in_bounds(0, bounds, visit);
        }
    }

    fn visit_in_bounds<F: FnMut(T, i32)>(&self, node: usize, bounds: &Bounds2D, visit: &mut F) {
        let n = &self.nodes[node];
        if n.has_items() {
            self.visit_node_items(node, visit);
        } else if n.has_children() {
            let first = n.first_child as usize;
            for c in first..first + 4 {
                if !self.node_empty(c) && bounds.overlaps(&self.nodes[c].bounds) {
                    self.visit_in_bounds(c, bounds, visit);
                }
            }
        }
    }

    /// `visitItemsAt(pos, visit)`.
    pub(crate) fn visit_items_at<F: FnMut(T, i32)>(&self, pos: Vec2, visit: &mut F) {
        let mut node = 0;
        if self.node_len == 0 {
            return;
        }
        loop {
            if self.nodes[node].has_items() {
                self.visit_node_items(node, visit);
                return;
            }
            match self.child_index(node, pos) {
                Some(c) => node = c,
                None => return,
            }
        }
    }

    /// `visitItemsInFrustum(frustum, visit)`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn visit_items_in_frustum<F: FnMut(T, i32)>(
        &self,
        frustum: &Frustum,
        visit: &mut F,
    ) {
        if self.node_len > 0 {
            self.visit_in_frustum(0, frustum, visit);
        }
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    fn visit_in_frustum<F: FnMut(T, i32)>(&self, node: usize, frustum: &Frustum, visit: &mut F) {
        let n = &self.nodes[node];
        if n.has_items() {
            self.visit_node_items(node, visit);
        } else if n.has_children() {
            let first = n.first_child as usize;
            for c in first..first + 4 {
                if !self.node_empty(c) && frustum.intersects(&self.nodes[c].bounds, 0.0) {
                    self.visit_in_frustum(c, frustum, visit);
                }
            }
        }
    }

    /// `getItemsAt(itemset, pos)`: appends every item of the leaf containing
    /// `pos` to `out` (with repeats, unsorted) and returns the C++ count.
    pub(crate) fn get_items_at(&self, out: &mut Vec<T>, pos: Vec2) -> i32 {
        let mut count = 0;
        self.visit_items_at(pos, &mut |id, _| {
            out.push(id);
            count += 1;
        });
        count
    }

    /// `getItemsInBounds(itemset, bounds)`: appends the items of every leaf
    /// reached through overlapping children and returns the C++ count.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn get_items_in_bounds(&self, out: &mut Vec<T>, bounds: &Bounds2D) -> i32 {
        let mut count = 0;
        self.visit_items_in_bounds(bounds, &mut |id, _| {
            out.push(id);
            count += 1;
        });
        count
    }

    /// `getItemsInFrustum(itemset, frustum)`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn get_items_in_frustum(&self, out: &mut Vec<T>, frustum: &Frustum) -> i32 {
        let mut count = 0;
        self.visit_items_in_frustum(frustum, &mut |id, _| {
            out.push(id);
            count += 1;
        });
        count
    }
}

impl<T: Copy + Ord> QuadTree<T> {
    /// The `std::set` of `getItemsAt`, as a sorted, deduplicated list in
    /// `out` (cleared first).
    pub(crate) fn collect_items_at(&self, out: &mut Vec<T>, pos: Vec2) {
        out.clear();
        self.get_items_at(out, pos);
        out.sort_unstable();
        out.dedup();
    }

    /// The `std::set` of `getItemsInBounds`, sorted and deduplicated.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn collect_items_in_bounds(&self, out: &mut Vec<T>, bounds: &Bounds2D) {
        out.clear();
        self.get_items_in_bounds(out, bounds);
        out.sort_unstable();
        out.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds2D {
        Bounds2D::new(Vec2::new(x0, y0), Vec2::new(x1, y1))
    }

    #[test]
    fn single_item_stays_in_root() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 6, 1);
        t.add_item(1u32, b(10.0, 10.0, 12.0, 12.0));
        assert_eq!(t.node_count, 1);
        assert_eq!(t.item_count, 1);
        assert_eq!(t.items()[0].node_count, 1);
    }

    #[test]
    fn second_item_splits_and_redistributes() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 6, 1);
        t.add_item(1u32, b(10.0, 10.0, 12.0, 12.0)); // top left
        t.add_item(2u32, b(80.0, 80.0, 82.0, 82.0)); // bottom right
        assert_eq!(t.node_count, 5);
        assert_eq!(t.item_count, 2);
        assert_eq!(t.unique_item_count, 2);

        let mut out = Vec::new();
        t.collect_items_at(&mut out, Vec2::new(11.0, 11.0));
        assert_eq!(out, vec![1]);
        t.collect_items_at(&mut out, Vec2::new(81.0, 81.0));
        assert_eq!(out, vec![2]);
        t.collect_items_at(&mut out, Vec2::new(81.0, 11.0));
        assert!(out.is_empty());
    }

    #[test]
    fn item_spanning_children_is_counted_per_leaf() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 2, 1);
        t.add_item(1u32, b(10.0, 10.0, 12.0, 12.0));
        // Covers all four quadrants.
        t.add_item(2u32, b(40.0, 40.0, 60.0, 60.0));
        assert_eq!(t.items()[1].node_count, 4);
        // Depth 2 is the max: the top-left leaf accepts both items.
        assert_eq!(t.items()[0].node_count, 1);
        assert_eq!(t.item_count, 5);

        let mut visits = Vec::new();
        t.visit_items_in_bounds(&b(0.0, 0.0, 100.0, 100.0), &mut |id, nc| {
            visits.push((id, nc))
        });
        // Leaf order TL, TR, BL, BR; insertion order inside a leaf.
        assert_eq!(visits, vec![(1, 1), (2, 4), (2, 4), (2, 4), (2, 4)]);

        let mut out = Vec::new();
        t.collect_items_in_bounds(&mut out, &b(0.0, 0.0, 100.0, 100.0));
        assert_eq!(out, vec![1, 2]);
    }

    #[test]
    fn max_depth_one_never_splits() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 1, 1);
        for i in 0..10u32 {
            t.add_item(i, b(i as f32, 0.0, i as f32 + 1.0, 1.0));
        }
        assert_eq!(t.node_count, 1);
        assert_eq!(t.item_count, 10);
        let mut visits = Vec::new();
        t.visit_items_in_bounds(&b(500.0, 500.0, 501.0, 501.0), &mut |id, _| visits.push(id));
        // The root's items are visited without any overlap test, as in C++.
        assert_eq!(visits, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn item_outside_every_child_is_dropped() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 6, 1);
        t.add_item(1u32, b(10.0, 10.0, 12.0, 12.0));
        t.add_item(2u32, b(500.0, 500.0, 501.0, 501.0));
        assert_eq!(t.items()[1].node_count, 0);
        assert_eq!(t.item_count, 1);
        assert_eq!(t.unique_item_count, 2);
    }

    #[test]
    fn reset_reuses_storage() {
        let mut t = QuadTree::new(b(0.0, 0.0, 100.0, 100.0), 6, 1);
        for i in 0..64u32 {
            let x = (i % 8) as f32 * 12.0;
            let y = (i / 8) as f32 * 12.0;
            t.add_item(i, b(x, y, x + 1.0, y + 1.0));
        }
        let nodes = t.node_count;
        let cap = (t.nodes.capacity(), t.items.capacity());
        t.reset(b(0.0, 0.0, 100.0, 100.0), 6, 1);
        assert_eq!(t.node_count, 1);
        for i in 0..64u32 {
            let x = (i % 8) as f32 * 12.0;
            let y = (i / 8) as f32 * 12.0;
            t.add_item(i, b(x, y, x + 1.0, y + 1.0));
        }
        assert_eq!(t.node_count, nodes);
        assert_eq!((t.nodes.capacity(), t.items.capacity()), cap);
    }
}
