import Rustlink.Width

namespace Rustlink

/-- Decoded link before bounds check (`crates/rustlink/src/link.rs:13-20`). -/
inductive Target where
  | null : Target
  | index : Nat → Target
  | invalid : Target
  deriving DecidableEq, Repr

/-- Four link payloads (`crates/rustlink/src/link.rs:97-247`). -/
inductive Link (w : Width) where
  | abs : Nat → Link w
  | optAbs : Nat → Link w
  | rel : Int → Link w
  | optRel : Int → Link w

/-- Signed-vs-unsigned collision check; rejects `None` self link. -/
def optRelConstructor (w : Width) (owner : Nat) (target : Option Nat) : Option (Link w) :=
  match target with
  | none => some (.optRel 0)
  | some t =>
    if t = owner then none
    else
      let d : Int := (t : Int) - (owner : Int)
      if d < w.signedMin ∨ w.signedMax < d then none
      else some (.optRel d)

/-- `owner : Nat`, delta applied in `Int`, then rechecked to `Nat`. -/
def relTarget (owner : Nat) (delta : Int) : Target :=
  match (owner : Int) + delta with
  | Int.ofNat n => .index n
  | _ => .invalid

def target (w : Width) (owner : Nat) : Link w → Target
  | .abs i => .index i
  | .optAbs stored => if stored = 0 then .null else .index (stored - 1)
  | .rel d => relTarget owner d
  | .optRel d => if d = 0 then .null else relTarget owner d

def check (w : Width) (owner : Nat) (len : Nat) (l : Link w) : Bool :=
  match target w owner l with
  | .null => true
  | .index i => i < len
  | .invalid => false

/-- Constructor-side invariants (`crates/rustlink/src/link.rs:99-133`,
    `152-195`): absolute links require `i ≤ MAX` and `OptAbs`
    requires `i + 1 ≤ MAX`. -/
def absConstructor (w : Width) (i : Nat) : Option (Link w) :=
  if i ≤ w.max then some (.abs i) else none

def optAbsConstructor (w : Width) (i : Option Nat) : Option (Link w) :=
  match i with
  | none => some (.optAbs 0)
  | some v => if v + 1 ≤ w.max then some (.optAbs (v + 1)) else none

theorem target_abs {w : Width} {i : Nat} :
    target w 0 (.abs i) = .index i := rfl

theorem target_optAbs_null {w : Width} :
    target w 0 (.optAbs 0) = .null := rfl

theorem target_optAbs_succ {w : Width} {v : Nat} :
    target w 0 (.optAbs (v + 1)) = .index v := by
  simp [target]

theorem check_abs {w : Width} {i len : Nat} :
    check w 0 len (.abs i) = decide (i < len) := rfl

end Rustlink
