//! Hidden-class "shapes" for objects — the JavaScriptCore *Structure* idea.
//!
//! A [`ShapeId`] is an interned identity for an object's **ordered string-key set**. Two objects
//! built by inserting the same keys in the same order share a `ShapeId`. This lets the bytecode VM's
//! inline caches (see `tish_bytecode::Chunk::inline_caches`) compare a single `u32` instead of hashing
//! a property name — on a shape hit the property is at a fixed slot index, so access is a direct load.
//!
//! Identity is **path-dependent** (like JSC): `{x,y}` and `{y,x}` are *different* shapes, because the
//! slot index of `x` differs — which is exactly what makes the cached `(shape, index)` correct.
//!
//! Phase 1a uses shapes only as opaque identities (the property→index lookup still goes through
//! `PropMap` on a cache miss). Phase 1b will attach the ordered key list to each shape so objects can
//! drop per-object key storage entirely (the butterfly representation).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Identity of an object's ordered key-set.
pub type ShapeId = u32;

/// The shape of a freshly-created empty object (`{}`).
pub const EMPTY_SHAPE: ShapeId = 0;

/// Sentinel for objects that have opted out of shape tracking (after a property *delete*, or when the
/// shape space is exhausted). Such objects never match an inline cache → always the slow path. Chosen
/// as `u32::MAX` so it can never collide with a real, sequentially-assigned id.
pub const DICT_SHAPE: ShapeId = u32::MAX;

/// One node in the structure-transition tree: from this shape, adding a given key yields a child shape.
#[derive(Default)]
struct ShapeNode {
    transitions: HashMap<Arc<str>, ShapeId>,
}

struct Registry {
    nodes: Vec<ShapeNode>,
}

fn registry() -> &'static RwLock<Registry> {
    static REG: OnceLock<RwLock<Registry>> = OnceLock::new();
    REG.get_or_init(|| {
        RwLock::new(Registry {
            // Index 0 == EMPTY_SHAPE.
            nodes: vec![ShapeNode::default()],
        })
    })
}

/// The shape reached by adding a **new** key `key` to an object currently of shape `from`.
///
/// Cached: the first object to take a given (shape, key) edge creates the child shape; every later
/// object with the same construction path reuses it. Cheap on the hot path (a read-lock + one hashmap
/// lookup once the edge exists). A `DICT_SHAPE` input (or shape-space exhaustion) stays `DICT_SHAPE`.
pub fn transition(from: ShapeId, key: &Arc<str>) -> ShapeId {
    if from == DICT_SHAPE {
        return DICT_SHAPE;
    }
    // Fast path: the edge already exists (the common case after the first object of this shape).
    {
        let reg = registry().read().unwrap();
        match reg.nodes.get(from as usize) {
            Some(node) => {
                if let Some(&next) = node.transitions.get(key.as_ref()) {
                    return next;
                }
            }
            None => return DICT_SHAPE, // out of range — should not happen; degrade safely
        }
    }
    // Slow path: create the child shape and cache the edge.
    let mut reg = registry().write().unwrap();
    // Re-check under the write lock (another thread may have created it meanwhile).
    if let Some(&next) = reg.nodes[from as usize].transitions.get(key.as_ref()) {
        return next;
    }
    let new_id = reg.nodes.len();
    if new_id >= DICT_SHAPE as usize {
        return DICT_SHAPE; // ran out of shape ids — extremely unlikely; degrade to dictionary mode
    }
    reg.nodes.push(ShapeNode::default());
    reg.nodes[from as usize]
        .transitions
        .insert(Arc::clone(key), new_id as ShapeId);
    new_id as ShapeId
}
