use std::path::PathBuf;

use vrf_bitio::BitReader;

const DEFAULT_REPLAY: &str = "02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf";

fn resolve_replay() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    if let Some(dir) = std::env::var_os("VRFKIT_CORPUS_DIR") {
        return PathBuf::from(dir).join(DEFAULT_REPLAY);
    }
    eprintln!(
        "usage: dump <path-to-replay.vrf>\n\
         \x20         or set VRFKIT_CORPUS_DIR to the corpus directory\n\
         \x20         (looks for {DEFAULT_REPLAY} inside it)"
    );
    std::process::exit(2);
}

fn main() {
    let path = resolve_replay();
    let data = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", path.display());
        std::process::exit(1);
    });

    // Skip to the header chunk payload
    // Info ends at offset 586, chunk header is 8 bytes, payload starts at 594
    let hp_start = 594usize;
    let chunk_size = i32::from_le_bytes(data[586 + 4..586 + 8].try_into().unwrap()) as usize;
    let payload = &data[hp_start..hp_start + chunk_size];

    let mut r = BitReader::new(payload);

    println!("Header payload size: {}", chunk_size);
    println!("NetworkMagic: 0x{:08X}", r.read_u32().unwrap());
    println!("NetworkVersion: {}", r.read_u32().unwrap());
    let cv_count = r.read_i32().unwrap();
    println!("CustomVersionCount: {}", cv_count);
    // Skip custom versions
    r.skip_bits((cv_count as u64) * 20 * 8).unwrap();
    println!("NetworkChecksum: 0x{:08X}", r.read_u32().unwrap());
    println!("EngineNetProtoVer: {}", r.read_u32().unwrap());
    println!("GameNetProtoVer: {}", r.read_u32().unwrap());
    // GUID
    let g = [
        r.read_u32().unwrap(),
        r.read_u32().unwrap(),
        r.read_u32().unwrap(),
        r.read_u32().unwrap(),
    ];
    println!("GUID: {:08X}-{:08X}-{:08X}-{:08X}", g[0], g[1], g[2], g[3]);
    // ReplayVersion
    println!("Major: {}", r.read_u16().unwrap());
    println!("Minor: {}", r.read_u16().unwrap());
    println!("Patch: {}", r.read_u16().unwrap());
    println!("Changelist: {}", r.read_u32().unwrap());
    let branch = r.read_fstring(65536).unwrap();
    println!("Branch: {}", branch);
    println!(
        "Position after branch: {} bits = {} bytes",
        r.position(),
        r.position() / 8
    );

    // ValorantSkipByteCount
    let skip_count = r.read_u32().unwrap();
    println!("ValorantSkipByteCount: {}", skip_count);
    r.skip_bits(skip_count as u64 * 8).unwrap();
    println!(
        "Position after valorant skip: {} bits = {} bytes",
        r.position(),
        r.position() / 8
    );

    // UE versions
    println!("UE4Version: {}", r.read_u32().unwrap());
    println!("UE5Version: {}", r.read_u32().unwrap());
    println!("PackageVersionLicense: {}", r.read_u32().unwrap());

    // LevelNamesAndTimes
    let level_count = r.read_i32().unwrap();
    println!("LevelCount: {}", level_count);
    for i in 0..level_count {
        let name = r.read_fstring(65536).unwrap();
        let time = r.read_u32().unwrap();
        println!("  Level[{}]: '{}' @ {}ms", i, name, time);
    }

    // Flags
    let flags = r.read_u32().unwrap();
    println!("Flags: 0x{:08X}", flags);

    // GameSpecificData
    let gsd_count = r.read_i32().unwrap();
    println!("GameSpecificDataCount: {}", gsd_count);
    for i in 0..gsd_count.min(10) {
        let s = r.read_fstring(65536).unwrap();
        println!("  GSD[{}]: '{}'", i, s);
    }

    println!(
        "Position after GSD: {} bits = {} bytes",
        r.position(),
        r.position() / 8
    );

    // Recording params
    println!("MinRecordHz: {}", r.read_f32().unwrap());
    println!("MaxRecordHz: {}", r.read_f32().unwrap());
    println!("FrameLimitInMs: {}", r.read_f32().unwrap());
    println!("CheckpointLimitInMs: {}", r.read_f32().unwrap());

    let platform = r.read_fstring(65536).unwrap();
    println!("Platform: '{}'", platform);
    println!("BuildConfig: {}", r.read_u8().unwrap());
    println!("BuildTargetType: {}", r.read_u8().unwrap());

    println!(
        "\nFinal position: {} bits = {} bytes of {} total",
        r.position(),
        r.position() / 8,
        chunk_size
    );
}
