import Rustlink.Link

namespace Rustlink

/-- `follow` over a list (`crates/rustlink/src/region.rs:30-35`). -/
def follow {α} (w : Width) (xs : List α) (owner : Nat) (l : Link w) : Option (Nat × α) :=
  match target w owner l with
  | .index i => xs[i]? |>.map fun a => (i, a)
  | .null | .invalid => none

/-- `try_follow`: null -> `ok none`, dangling -> error. -/
def tryFollow {α} (w : Width) (xs : List α) (owner : Nat) (l : Link w) : Except Nat (Option (Nat × α)) :=
  match target w owner l with
  | .null => .ok none
  | .index i => match xs[i]? with
    | some a => .ok (some (i, a))
    | none => .error owner
  | .invalid => .error owner

/-- Soundness: any successful follow is inside the region. -/
theorem follow_sound {α} (w : Width) (xs : List α) (owner : Nat) (l : Link w)
    {i : Nat} {a : α}
    (h : follow w xs owner l = some (i, a)) : i < xs.length := by
  unfold follow at h
  cases ht : target w owner l with
  | null => simp [ht] at h
  | invalid => simp [ht] at h
  | index j =>
    rw [ht] at h
    simp at h
    obtain ⟨ha, hj⟩ := h
    rw [hj] at ha
    obtain ⟨hin, _⟩ := List.getElem?_eq_some_iff.mp ha
    exact hin

theorem tryFollow_sound {α} (w : Width) (xs : List α) (owner : Nat) (l : Link w)
    {i : Nat} {a : α}
    (h : tryFollow w xs owner l = .ok (some (i, a))) : i < xs.length := by
  unfold tryFollow at h
  cases ht : target w owner l with
  | null => simp [ht] at h
  | invalid => simp [ht] at h
  | index j =>
    rw [ht] at h
    cases hx : xs[j]? with
    | none => simp [hx] at h
    | some b =>
      simp [hx] at h
      obtain ⟨hj, _⟩ := h
      rw [hj] at hx
      obtain ⟨hin, _⟩ := List.getElem?_eq_some_iff.mp hx
      exact hin

end Rustlink
