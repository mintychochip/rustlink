namespace Rustlink

/-- Storage width of a link offset (`crates/rustlink/src/width.rs:13-42`). -/
structure Width where
  bytes : Nat
  max : Nat
  signedMin : Int
  signedMax : Int

def w8 : Width := ⟨1, 255, -128, 127⟩
def w16 : Width := ⟨2, 65535, -32768, 32767⟩
def w32 : Width := ⟨4, 4294967295, -2147483648, 2147483647⟩
def w64 : Width := ⟨8, 18446744073709551615, -9223372036854775808, 9223372036854775807⟩

/-- `2^(8·b) - 1`. -/
def maxFor (b : Nat) : Nat := 2 ^ (8 * b) - 1

theorem w8_max : w8.max = maxFor w8.bytes := by decide
theorem w16_max : w16.max = maxFor w16.bytes := by decide
theorem w32_max : w32.max = maxFor w32.bytes := by decide
theorem w64_max : w64.max = maxFor w64.bytes := by decide

/-- Concrete signed spans (`SIGNED_MAX - SIGNED_MIN + 1`). -/
theorem w8_span : w8.signedMax - w8.signedMin + 1 = 256 := by decide
theorem w16_span : w16.signedMax - w16.signedMin + 1 = 65536 := by decide
theorem w32_span : w32.signedMax - w32.signedMin + 1 = 4294967296 := by decide
theorem w64_span : w64.signedMax - w64.signedMin + 1 = (2 ^ 64 : Int) := by decide

/-- Truncated encode/decode: values in range round-trip. -/
def encode (w : Width) (raw : Nat) : Nat := if raw ≤ w.max then raw else 0
def decode (_w : Width) (stored : Nat) : Nat := stored

theorem encode_decode (w : Width) {raw : Nat} (h : raw ≤ w.max) :
    decode w (encode w raw) = raw := by
  simp [encode, decode, h]

theorem decode_le_max (w : Width) {stored : Nat} (h : stored ≤ w.max) :
    decode w stored ≤ w.max := h

/-- `node_limit`: stored value is index+1, so capacity is `MAX`. -/
def nodeLimit (w : Width) : Nat := w.max

end Rustlink
