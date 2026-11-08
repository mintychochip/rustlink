import Rustlink.Link

namespace Rustlink

/-- Largest region addressable by `Abs` at width `w`: indices `0 ..= MAX`. -/
def absCapacity (w : Width) : Nat := w.max + 1

/-- Largest region addressable by `OptAbs`: indices `0 ..< MAX` (null excluded). -/
def optAbsCapacity (w : Width) : Nat := w.max

/-- Distinct `Rel` deltas (`SIGNED_MAX - SIGNED_MIN + 1`). -/
def relSpan (w : Width) : Nat := Int.toNat (w.signedMax - w.signedMin + 1)

/-- Non-null `OptRel` deltas: span minus the self-collision `0`. -/
def optRelSpan (w : Width) : Nat := relSpan w - 1

/-- Largest element count a container with this width can hold. -/
def containerCap (w : Width) : Nat := nodeLimit w

/-- Well-formedness: stored index/delta inside the width's range. -/
def wf (w : Width) : Link w → Bool
  | .abs i => decide (i ≤ w.max)
  | .optAbs s => decide (s ≤ w.max)
  | .rel d => decide (w.signedMin ≤ d ∧ d ≤ w.signedMax)
  | .optRel d => decide (d = 0 ∨ (w.signedMin ≤ d ∧ d ≤ w.signedMax))

theorem wf_absConstructor (w : Width) (i : Nat) (l : Link w)
    (h : absConstructor w i = some l) : wf w l = true := by
  unfold absConstructor at h
  by_cases hi : i ≤ w.max
  · simp [hi] at h
    obtain rfl : l = .abs i := h.symm
    simp [wf, hi]
  · simp [hi] at h

theorem absConstructor_some_iff (w : Width) (i : Nat) :
    absConstructor w i ≠ none ↔ i < absCapacity w := by
  unfold absConstructor
  by_cases hi : i ≤ w.max <;> simp [hi, absCapacity] <;> omega

theorem optAbsConstructor_some_iff (w : Width) (v : Nat) :
    optAbsConstructor w (some v) ≠ none ↔ v < optAbsCapacity w := by
  unfold optAbsConstructor
  by_cases hi : v + 1 ≤ w.max <;> simp [hi, optAbsCapacity] <;> omega

theorem absConstructor_target (w : Width) (owner : Nat) (i : Nat) (l : Link w)
    (h : absConstructor w i = some l) :
    target w owner l = .index i := by
  unfold absConstructor at h
  by_cases hi : i ≤ w.max
  · simp [hi] at h
    obtain rfl : l = .abs i := h.symm
    rfl
  · simp [hi] at h

theorem target_abs_any (w : Width) (owner : Nat) (i : Nat) :
    target w owner (.abs i) = .index i := rfl

theorem abs_w8 : absCapacity w8 = 256 := by decide
theorem abs_w16 : absCapacity w16 = 65536 := by decide
theorem abs_w32 : absCapacity w32 = 4294967296 := by decide

theorem optAbs_w8 : optAbsCapacity w8 = 255 := by decide
theorem optAbs_w16 : optAbsCapacity w16 = 65535 := by decide
theorem optAbs_w32 : optAbsCapacity w32 = 4294967295 := by decide

theorem relSpan_w16 : relSpan w16 = 65536 := by decide
theorem optRel_w16 : optRelSpan w16 = 65535 := by decide

theorem abs_gt_optAbs (w : Width) : absCapacity w = optAbsCapacity w + 1 := rfl

theorem abs_eq_pow (w : Width) (h : w.max = maxFor w.bytes) :
    absCapacity w = 2 ^ (8 * w.bytes) := by
  unfold absCapacity maxFor at *
  rw [h]
  have hpos : 0 < 2 ^ (8 * w.bytes) := Nat.pow_pos (by decide)
  omega

theorem optAbs_eq_nodeLimit (w : Width) : optAbsCapacity w = nodeLimit w := rfl

end Rustlink
