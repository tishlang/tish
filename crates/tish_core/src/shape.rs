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

use crate::compat::Arc;
#[cfg(not(feature = "portable"))]
use crate::compat::{AHashMap, OnceLock, RwLock};

/// Identity of an object's ordered key-set.
pub type ShapeId = u32;

/// The shape of a freshly-created empty object (`{}`).
pub const EMPTY_SHAPE: ShapeId = 0;

/// Sentinel for objects that have opted out of shape tracking (after a property *delete*, or when the
/// shape space is exhausted). Such objects never match an inline cache → always the slow path. Chosen
/// as `u32::MAX` so it can never collide with a real, sequentially-assigned id.
pub const DICT_SHAPE: ShapeId = u32::MAX;

/// One node in the structure-transition tree: from this shape, adding a given key yields a child shape.
/// Edges are keyed by an `ahash` map, not the default SipHash one: `transition` is on the hot path of
/// every object construction (and `JSON.parse` — profiled as ~16% of parse, almost all SipHash), and
/// ahash is ~2–3× faster on short string keys. Random-seeded per process, so no HashDoS regression.
/// Growth backstops (#701/#706/#710). The registry has no eviction — a permanent `ShapeNode`
/// plus an `Arc` clone of the key per distinct `(parent, key)` edge — so every mint site must be
/// bounded. Three independent caps, all degrading to `DICT_SHAPE` (never an error):
///
/// - `MAX_TRANSITIONS_PER_NODE` bounds a node's fan-out. The hot pathology is node 0: every
///   fresh `{}` that takes a data-dependent first key (`o[requestId] = …`) adds one edge to
///   EMPTY_SHAPE's map, forever, under the global write lock.
/// - `MAX_CHAIN_DEPTH` bounds a chain's length. A single object accumulating keys one at a time
///   mints a linear chain (each node fan-out 1, so the fan-out cap never trips). `PropMap`
///   promotion demotes to dictionary mode at 9 keys, but `with_capacity`-built maps skip
///   promotion — the depth cap is their backstop.
/// - `MAX_SHAPES` is the absolute registry ceiling; past it nothing new is ever interned.
///
/// Values are far above anything a program's TEXT can mint (shapes are meant to be bounded by
/// code structure), and far below the ~750 GB where the u32 id-overflow guard would trip.
const MAX_TRANSITIONS_PER_NODE: usize = 256;
const MAX_CHAIN_DEPTH: u16 = 32;
const MAX_SHAPES: usize = 1 << 18; // 262_144 nodes ≈ tens of MB worst-case, orders above real use

#[cfg(not(feature = "portable"))]
#[derive(Default)]
struct ShapeNode {
    transitions: AHashMap<Arc<str>, ShapeId>,
    /// Distance from EMPTY_SHAPE (== number of keys on the path). Bounds chain length.
    depth: u16,
}

#[cfg(not(feature = "portable"))]
struct Registry {
    nodes: Vec<ShapeNode>,
}

#[cfg(not(feature = "portable"))]
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
    // On `portable` (the embedded/GBA link) nothing ever READS a shape id: the only consumers are
    // the bytecode VM (`tish_vm`) and the std runtime's inline caches (`tish_runtime`), and neither
    // is linked there — the GBA facade's `get_prop` goes straight to `PropMap`. Maintaining the
    // registry anyway cost a lock plus an ahash lookup on every new-key insert AND — since this
    // file has no eviction of any kind — a permanent `ShapeNode` plus an `Arc::clone` of the key
    // for every distinct construction path, for the life of the program, on a 136 KB heap.
    // Opt out via the sentinel the file already defines for exactly this case: `DICT_SHAPE` never
    // matches an inline cache, so any future reader degrades to the slow path rather than trusting
    // a slot index that was never recorded.
    #[cfg(feature = "portable")]
    {
        let _ = (from, key);
        DICT_SHAPE
    }
    #[cfg(not(feature = "portable"))]
    {
        transition_tracked(from, key)
    }
}

#[cfg(not(feature = "portable"))]
fn transition_tracked(from: ShapeId, key: &Arc<str>) -> ShapeId {
    if from == DICT_SHAPE {
        return DICT_SHAPE;
    }
    // Fast path: the edge already exists (the common case after the first object of this shape).
    // The caps are ALSO checked here, read-only: a saturated node keeps answering DICT_SHAPE
    // from the read lock instead of convoying every miss through the write lock below.
    {
        let reg = registry().read().unwrap();
        match reg.nodes.get(from as usize) {
            Some(node) => {
                if let Some(&next) = node.transitions.get(key.as_ref()) {
                    return next;
                }
                if node.transitions.len() >= MAX_TRANSITIONS_PER_NODE
                    || node.depth >= MAX_CHAIN_DEPTH
                {
                    return DICT_SHAPE;
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
    // Re-check the caps under the write lock too (the read-path check can race).
    let parent_depth = reg.nodes[from as usize].depth;
    if reg.nodes[from as usize].transitions.len() >= MAX_TRANSITIONS_PER_NODE
        || parent_depth >= MAX_CHAIN_DEPTH
        || reg.nodes.len() >= MAX_SHAPES
    {
        return DICT_SHAPE;
    }
    let new_id = reg.nodes.len();
    if new_id >= DICT_SHAPE as usize {
        return DICT_SHAPE; // id-space overflow guard — prevents u32 truncation aliasing live ids
    }
    reg.nodes.push(ShapeNode {
        transitions: AHashMap::default(),
        depth: parent_depth + 1,
    });
    reg.nodes[from as usize]
        .transitions
        .insert(Arc::clone(key), new_id as ShapeId);
    new_id as ShapeId
}

/// Number of interned shape nodes — for regression tests asserting that a workload does NOT
/// grow the registry. Not part of the public surface.
#[doc(hidden)]
#[cfg(not(feature = "portable"))]
pub fn shape_count() -> usize {
    registry().read().unwrap().nodes.len()
}

#[doc(hidden)]
#[cfg(feature = "portable")]
pub fn shape_count() -> usize {
    0
}
