namespace Rustlink

/-- Append-only arena as a list (`crates/rustlink/src/arena.rs:50-67`). -/
def push (xs : List α) (x : α) : List α × Nat := (xs ++ [x], xs.length)

/-- Earlier indices do not move. -/
theorem push_preserves {α} (xs : List α) (x : α) {i : Nat} (h : i < xs.length) :
    (push xs x).1[i]? = xs[i]? := by
  show (xs ++ [x])[i]? = xs[i]?
  rw [List.getElem?_append, ite_eq_left h]

theorem push_fresh {α} (xs : List α) (x : α) :
    (push xs x).2 = xs.length := rfl

theorem push_len {α} (xs : List α) (x : α) : (push xs x).1.length = xs.length + 1 := by
  show (xs ++ [x]).length = xs.length + 1
  simp
/-- `clear` empties the arena (`crates/rustlink/src/arena.rs:75-77`). -/
def clear (_xs : List α) : List α := []

/-- After `clear`, every previously valid index is now invalid. -/
theorem clear_invalidates {α} (xs : List α) {i : Nat} (_hi : i < xs.length) :
    ¬ (i < (clear xs).length) := by
  simp [clear]

end Rustlink
