import Rustlink.Link
import Rustlink.Region

namespace Rustlink

/-- Blob image length (`crates/rustlink/src/blob.rs:9,101-112`). -/
def blobLength (count : Nat) (elemSize : Nat) : Nat := 32 + count * elemSize

/-- Exact-length check: the header-implied length matches the total. -/
def lengthOk (total : Nat) (count : Nat) (elemSize : Nat) : Prop :=
  total = blobLength count elemSize

/-- Payload bytes are exactly `count * elemSize`. -/
theorem lengthOk_payload (total count elemSize : Nat)
    (h : lengthOk total count elemSize) : total - 32 = count * elemSize := by
  simp [lengthOk, blobLength] at h
  omega

theorem blobLength_empty (elemSize : Nat) : blobLength 0 elemSize = 32 := by
  simp [blobLength]

/-- Decoded header fields (`blob.rs:78-101`). -/
structure Header where
  magicOk : Bool
  versionOk : Bool
  elemSizeOk : Bool
  schemaOk : Bool
  count : Nat
  elemSize : Nat

/-- Count-overflow check (`blob.rs:103-106`): `count * elemSize + 32`
    fits in `u64` (`2 ^ 64 - 1`). -/
def countNoOverflow (count elemSize : Nat) : Bool :=
  decide (count * elemSize + 32 ≤ 2 ^ 64 - 1)

/-- `verify` short-circuit order (`blob.rs:78-121`): truncation,
    magic, version, elem size, schema, count-overflow, exact length,
    then per-element links. -/
def verifyFull (w : Width) (total : Nat) (hdr : Header)
    (links : Nat → List (Link w)) : Bool :=
  hdr.magicOk && hdr.versionOk && hdr.elemSizeOk && hdr.schemaOk &&
    countNoOverflow hdr.count hdr.elemSize &&
    decide (total = blobLength hdr.count hdr.elemSize) &&
    decide (∀ i, i < hdr.count →
      ∀ l ∈ links i, check w i hdr.count l = true)

/-- Soundness: a `verifyFull` pass implies every same-region link resolves. -/
theorem verifyFull_sound (w : Width) (total : Nat) (hdr : Header)
    (links : Nat → List (Link w))
    (h : verifyFull w total hdr links = true) (i : Nat) (hi : i < hdr.count)
    (l : Link w) (hl : l ∈ links i) :
    check w i hdr.count l = true := by
  simp [verifyFull] at h
  exact h.right i hi l hl

/-- Bridge to `tryFollow`: a checked link never yields `.error`. -/
theorem check_no_error {α} (w : Width) (xs : List α) (i : Nat) (l : Link w)
    (hc : check w i xs.length l = true) :
    ∀ r, tryFollow w xs i l ≠ .error r := by
  intro r h
  unfold check at hc
  unfold tryFollow at h
  cases ht : target w i l with
  | null => simp [ht] at h
  | invalid => simp [ht] at hc
  | index j =>
    cases hx : xs[j]? with
    | none =>
      have hc' : j < xs.length := by
        simpa [ht] using hc
      have hle : xs.length ≤ j := (List.getElem?_eq_none_iff).mp hx
      omega
    | some a => simp [ht, hx] at h

/-- A `verifyFull` pass implies every listed link is checked, hence
    `tryFollow` never reports `.error` for those links. -/
theorem verifyFull_tryFollow_ne_error {α} (w : Width) (xs : List α)
    (total : Nat) (hdr : Header) (links : Nat → List (Link w))
    (h : verifyFull w total hdr links = true)
    (hlen : xs.length = hdr.count)
    (i : Nat) (l : Link w) (hi : i < hdr.count) (hl : l ∈ links i) :
    ∀ r, tryFollow w xs i l ≠ .error r := by
  intro r
  have hc : check w i hdr.count l = true :=
    verifyFull_sound w total hdr links h i hi l hl
  rw [← hlen] at hc
  exact check_no_error w xs i l hc r

theorem verified_implies_valid_length (total count elemSize : Nat)
    (h : lengthOk total count elemSize) : total - 32 = count * elemSize :=
  lengthOk_payload total count elemSize h

end Rustlink
