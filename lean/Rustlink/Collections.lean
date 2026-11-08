import Rustlink.Width

namespace Rustlink

/-- All containers store nodes in one arena `Vec` and refer through
    `OptAbs` links, so the node count is capped at `nodeLimit = MAX`
    (`crates/rustlink-collections/src/limit.rs:5-9`). -/
def listCap (w : Width) : Nat := nodeLimit w
def hashCap (w : Width) : Nat := nodeLimit w

/-- B+tree leaf capacity (`crates/rustlink-collections/src/btree/mod.rs`). -/
def leafCap : Nat := 128

/-- Upper bound on entries: every entry lives in a leaf, leaves ≤ nodes,
    so `entries ≤ 128 * MAX`. Unreachable: inner nodes share the same `MAX`
    budget (`leaves ≤ MAX - innerNodes`) and leaves split at `MIN_LEAF = 64`.
    `README.md` measures ~5.7M entries for `u16`, not 8,388,480. -/
def btreeCap (w : Width) : Nat := leafCap * nodeLimit w

theorem listCap_w16 : listCap w16 = 65535 := by decide
theorem hashCap_w16 : hashCap w16 = 65535 := by decide
theorem btreeCap_w16 : btreeCap w16 = 8388480 := by decide

theorem listCap_eq : (listCap w) = (hashCap w) := rfl

theorem btreeCap_ge_list (w : Width) : btreeCap w ≥ listCap w := by
  unfold btreeCap listCap leafCap
  omega

/-- Pure linkage saving: pointer link (8 B) vs narrow link (`w.bytes` B). -/
def maxSaving (w : Width) : Nat := 8 / w.bytes

theorem maxSaving_w32 : maxSaving w32 = 2 := by decide
theorem maxSaving_w16 : maxSaving w16 = 4 := by decide
theorem maxSaving_w8 : maxSaving w8 = 8 := by decide

/-- Node size: `payload + k * w` vs pointer `payload + 8 * k`. -/
def nodeSize (w : Width) (payload k : Nat) : Nat := payload + k * w.bytes

theorem nodeSize_le_pointers (w : Width) (payload k : Nat) (h : w.bytes ≤ 8) :
    nodeSize w payload k ≤ payload + 8 * k := by
  unfold nodeSize
  have : k * w.bytes ≤ k * 8 := by exact Nat.mul_le_mul_left k h
  omega

end Rustlink
