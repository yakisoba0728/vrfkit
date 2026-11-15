// Fixed 13.10 boundary vectors for the replay payload transform.
// The 1, 7, 8, 31 and 32-bit cases were checked against the native
// FBitReader routine at RVA 0x432e600 in the 13.10 executable (SHA-256
// 8e6f270fd509b3996685ef986423866796b7b4db12f41eb51c769e50a99a1372).
// The larger cases cover regressions in the disassembled 64 -> 32 -> 8 -> tail
// implementation; a full 13.10 replay independently passes the block oracle.
// Tuple: bit count, expected output for PAYLOAD_HEX with seed = bits ^ 2.
pub const RECOVERED_1310_VECTORS: &[(usize, &str)] = &[
    (0, ""),
    (1, "01"),
    (7, "09"),
    (8, "E3"),
    (31, "81A90E6C"),
    (32, "9BEEED3B"),
    (63, "3DD567DBA74FD53B"),
    (64, "940242C04DB53A20"),
    (65, "B92B20E41756AB0301"),
    (287, "D37FF5963920BD0A120D371387A22E050731A05A8A754AB9C26C98457040FC56A9A05B18"),
    (288, "545F1060C4825F097D33D4CC4B4145FD75921C9477F375A61FF101CDFB56EA151354CFED"),
];
