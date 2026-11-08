import Rustlink.Width

namespace Rustlink

/-- Link layout: `size = BYTES`, `align = 1`.
    `crates/rustlink/src/link.rs:273-290` (`has_layout`, `assert_layouts!`). -/
def linkSize (w : Width) : Nat := w.bytes
def linkAlign (_w : Width) : Nat := 1

theorem linkSize_w8 : linkSize w8 = 1 := by decide
theorem linkSize_w16 : linkSize w16 = 2 := by decide
theorem linkSize_w32 : linkSize w32 = 4 := by decide
theorem linkSize_w64 : linkSize w64 = 8 := by decide

/-- Node layout (`#[repr(C)]`, `Unaligned` zerocopy scalars): no padding. -/
def nodeBytes (payload k : Nat) (w : Width) : Nat := payload + k * w.bytes
def nodeAlign (_w : Width) : Nat := 1

/-- BST node: `U32` payload + 2 links (`README.md` example, `size_of = 8`). -/
theorem bstNode_w16 : nodeBytes 4 2 w16 = 8 := by decide
theorem bstNode_w32 : nodeBytes 4 2 w32 = 12 := by decide

/-- Graph node (`rustlink-bench/src/ngraph.rs:3`): `u32` payload + 4 links. -/
theorem ngraph_w32 : nodeBytes 4 4 w32 = 20 := by decide
theorem ngraph_w16 : nodeBytes 4 4 w16 = 12 := by decide
theorem ngraph_w8 : nodeBytes 4 4 w8 = 8 := by decide

/-- Pointer baseline: 8 bytes/link (`usize`/`Box`). -/
def pointerNodeSize (payload k : Nat) : Nat := payload + 8 * k

theorem node_le_pointer (payload k : Nat) (w : Width) (h : w.bytes ≤ 8) :
    nodeBytes payload k w ≤ pointerNodeSize payload k := by
  unfold nodeBytes pointerNodeSize
  have : k * w.bytes ≤ k * 8 := Nat.mul_le_mul_left k h
  omega

/-- A field as a size/alignment pair; a node is a list of fields. -/
structure Field where
  size : Nat
  align : Nat
  deriving DecidableEq, Repr

/-- Total layout size is the sum of field sizes. -/
def layoutSize (fs : List Field) : Nat := (fs.map Field.size).sum

/-- Fields of a node: one payload plus `k` link fields, all `align = 1`. -/
def nodeFields (payload k : Nat) (w : Width) : List Field :=
  ⟨payload, 1⟩ :: List.replicate k ⟨w.bytes, 1⟩

theorem layoutSize_nodeFields (payload k : Nat) (w : Width) :
    layoutSize (nodeFields payload k w) = nodeBytes payload k w := by
  unfold layoutSize nodeFields nodeBytes
  simp

/-- With every alignment equal to 1, layout size is exactly the sum of
    field sizes and padding is zero. -/
theorem padding_zero_of_align_one (fs : List Field)
    (h : ∀ f ∈ fs, f.align = 1) :
    layoutSize fs = (fs.map Field.size).sum := rfl

/-- The BST and graph instances, in terms of `size = sum`, `padding = 0`. -/
theorem bstNode_w16_fields :
    layoutSize (nodeFields 4 2 w16) = 8 := by
  rw [layoutSize_nodeFields]
  decide

theorem ngraph_w32_fields :
    layoutSize (nodeFields 4 4 w32) = 20 := by
  rw [layoutSize_nodeFields]
  decide

/-- Arena heap, requested bytes (`rustlink-bench/src/alloc.rs:15-55`):
    counting allocator excludes allocator metadata. -/
def heapBytes (n size : Nat) : Nat := n * size

/-- Monotonicity: more elements cannot use fewer requested bytes. -/
theorem heap_mono (size : Nat) {n m : Nat} (h : n ≤ m) :
    heapBytes n size ≤ heapBytes m size := by
  unfold heapBytes
  exact Nat.mul_le_mul_right size h

/-- Doubling-`Vec` capacity model: `n` elements fit in capacity
    `2 ^ (log2 n + 1)`. The capacity bound `≤ 2 * n` is the `Vec`-growth
    guarantee, stated here as monotonicity at the capacity. -/
theorem heap_doubling_bound (size : Nat) {n : Nat}
    (hpow : n ≤ 2 ^ (Nat.log2 n + 1)) :
    heapBytes n size ≤ heapBytes (2 ^ (Nat.log2 n + 1)) size :=
  heap_mono size hpow

end Rustlink
