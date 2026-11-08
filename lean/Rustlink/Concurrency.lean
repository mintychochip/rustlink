import Rustlink.Link
import Rustlink.Region

namespace Rustlink

/-- Snapshot purity: appending one element cannot change the result of a
    follow whose target already resolved in the snapshot. -/
theorem follow_append_preserves_hit {α} (w : Width) (xs : List α) (x : α)
    (o : Nat) (l : Link w) {i : Nat} {a : α}
    (h : follow w xs o l = some (i, a)) :
    follow w (xs ++ [x]) o l = some (i, a) := by
  unfold follow at h ⊢
  cases ht : target w o l with
  | null => simp [ht] at h
  | invalid => simp [ht] at h
  | index j =>
    simp [ht] at h ⊢
    obtain ⟨ha, hj⟩ := h
    have hi : j < xs.length := by
      obtain ⟨hin, _⟩ := List.getElem?_eq_some_iff.mp ha
      exact hin
    subst hj
    rw [List.getElem?_append_left hi, ha]
    exact ⟨rfl, rfl⟩

theorem tryFollow_append_preserves_hit {α} (w : Width) (xs : List α) (x : α)
    (o : Nat) (l : Link w) {i : Nat} {a : α}
    (h : tryFollow w xs o l = .ok (some (i, a))) :
    tryFollow w (xs ++ [x]) o l = .ok (some (i, a)) := by
  unfold tryFollow at h ⊢
  cases ht : target w o l with
  | null => simp [ht] at h
  | invalid => simp [ht] at h
  | index j =>
    simp [ht] at h ⊢
    cases hx : xs[j]? with
    | none => simp [hx] at h
    | some b =>
      simp [hx] at h
      obtain ⟨hj, hb⟩ := h
      have hi : j < xs.length := by
        obtain ⟨hin, _⟩ := List.getElem?_eq_some_iff.mp hx
        exact hin
      subst hj
      subst hb
      rw [List.getElem?_append_left hi, hx]

/-- Reader-writer refinement: a reader holding `xs` sees the same answers
    for every index `< xs.length` after any sequence of pushes. -/
theorem follow_append_tail_preserves_hit {α} (w : Width) (xs tail : List α)
    (o : Nat) (l : Link w) {i : Nat} {a : α}
    (h : follow w xs o l = some (i, a)) :
    follow w (xs ++ tail) o l = some (i, a) := by
  unfold follow at h ⊢
  cases ht : target w o l with
  | null => simp [ht] at h
  | invalid => simp [ht] at h
  | index j =>
    simp [ht] at h ⊢
    obtain ⟨ha, hj⟩ := h
    have hi : j < xs.length := by
      obtain ⟨hin, _⟩ := List.getElem?_eq_some_iff.mp ha
      exact hin
    subst hj
    rw [List.getElem?_append_left hi, ha]
    exact ⟨rfl, rfl⟩

/-- A link of an older element still resolves after a push: `check` stays
    `true` once the target was in range. -/
theorem check_stable_after_push {α} (w : Width) (xs : List α) (x : α)
    (o : Nat) (l : Link w)
    (h : check w o xs.length l = true)
    (hind : ∀ i, target w o l = .index i → i < xs.length) :
    check w o (xs ++ [x]).length l = true := by
  unfold check at h ⊢
  cases ht : target w o l with
  | null => simp [ht]
  | invalid => simp [ht] at h
  | index i =>
    simp [ht] at h ⊢
    have hi : i < xs.length := hind i ht
    omega

end Rustlink
