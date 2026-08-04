# vrfkit 사용법

CLI, 출력 스키마, 크레이트별 사용, `tools/` 스크립트, 검증 스위트.

설계 근거와 기존 파서 대조 결과는 [`../README.md`](../README.md), 작업 이력과 측정
기록은 [`../PROJECT_STATUS.md`](../PROJECT_STATUS.md)에 있습니다.

---

## 1. 빌드

```bash
cargo build --release -p vrfkit                     # inspect / validate
cargo build --release -p vrfkit --features export   # + export (Parquet)
```

`export`는 기본 기능이 아닙니다. 빼고 빌드하면 arrow/parquet/zstd가 의존성 트리에
아예 들어오지 않습니다:

```bash
cargo tree -p vrfkit --no-default-features | grep -E "arrow|parquet|zstd"   # 출력 없음
```

`export` 없이 빌드된 바이너리에 `export`를 주면 **조용히 성공하지 않고 거부**합니다.
아무것도 안 쓰고 0으로 끝나는 건 파일을 쓴 것과 구분되지 않기 때문입니다.

---

## 2. CLI

```
vrfkit inspect  <file.vrf>
vrfkit validate <file.vrf> [--diagnostics]
vrfkit export   <file.vrf> --out <dir> [--checkpoints]
```

### `inspect` — 파일이 뭔지 본다

ReplayInfo, 헤더, 브랜치, 청크 요약을 출력합니다. 파싱은 하지 않으므로 즉시 끝납니다.
빌드가 지원 대상인지, 암호화됐는지 먼저 확인할 때 씁니다.

```
=== Header ===
  Replay version:   5.3.2 (changelist 2152699011)
  Branch:           ++Ares-Core+release-13.02
  Platform:         LinuxServer
=== Chunks ===
  ReplayData:       23 chunks (55297993 bytes)
  Checkpoint:       22 chunks
  Event:           238 chunks
```

### `validate` — 문법 오라클

모든 콘텐츠 블록을 RepLayout 문법으로 걸어보고 통과율을 보고합니다. 파일을 쓰지
않습니다.

```
  Total content blocks: 743110
  Malformed framing:  0          <- 프레이밍이 어긋난 블록. 0이 아니면 심각
  Transform failed:   0          <- 페이로드 변환 실패. 빌드 미지원 신호
  RPC stream failed:  7889       <- 그룹을 특정 못 해 handle 폭을 모르는 블록
  ORACLE PASS RATE:     98.938381%
```

**통과율이 100%가 아닌 것은 정상입니다.** 프레이밍 문제가 아니라 귀속 문제입니다 —
블록은 정확히 잘라내지만 일부는 어느 `_ClassNetCache` 그룹 소속인지 확정할 수
없습니다. 봐야 할 줄은 `Malformed framing`과 `Transform failed`이고, 둘 다 0이어야
합니다.

`--diagnostics`는 실패 블록 전체의 컨텍스트를 출력합니다. 기본값은 32줄까지만
보여주고 총계/표시/생략 개수를 헤더에 찍습니다.

### `export` — Parquet 내보내기

```bash
vrfkit export replay.vrf --out out/
vrfkit export replay.vrf --out out/ --checkpoints
```

`--checkpoints`는 Checkpoint 청크까지 읽어 `checkpoint_fields.parquet`를 **추가로**
씁니다. 기본 off인 이유는 파일의 약 10%를 더 읽는 별도 패스이기 때문이고,
**켜든 끄든 나머지 다섯 테이블은 바이트 단위로 동일합니다.**

#### 요약에서 실제로 봐야 할 줄

```
  Malformed pkts:   0        <- 0이 아니면 프레이밍이 깨진 것
  Struct blobs:     207 decoded / 0 failed
  Decode errors:    0        <- 0이 아니면 오버레이 타입이 틀린 것
```

`Struct blobs`는 `RoundResults` / `TeamEconomy` / `RoundInfos` 전용 디코더의 결과입니다.
이 디코더들은 **additive**라서 완전히 실패해도 다른 카운터가 하나도 안 움직입니다 —
빌드 13.02가 핸들을 밀었을 때 요약 전체가 정상으로 보이면서 경기 점수만 사라진 적이
있습니다(PROJECT_STATUS 26절). **`0 decoded`는 `failed`가 0이어도 경보입니다.**
실패 시 `Struct blob err:` 줄에 멤버와 핸들이 이름으로 찍힙니다.

---

## 3. 출력

`02d4d478`(48,215,213바이트) 기준 실측:

| 파일 | 행 | 바이트 |
|---|---|---|
| `fields.parquet` | 1,246,812 | 13,742,379 |
| `movement.parquet` | 1,839,607 | 31,835,557 |
| `actors.parquet` | 3,827 | 87,281 |
| `net_guids.parquet` | 16,167 | 153,606 |
| `events.parquet` | 195 | 10,201 |
| `checkpoint_fields.parquet` | 78,748 | 191,335 | `--checkpoints` 필요 |
| `manifest.json` | | 658,918 |

export 0.83초. 문자열 컬럼은 딕셔너리 인코딩 + ZSTD입니다.

### `fields.parquet` — 리플리케이션된 프로퍼티와 RPC 파라미터

| 컬럼 | 타입 | 설명 |
|---|---|---|
| `time_ms` | u32 | 리플레이 시작 기준 밀리초 |
| `packet_id` | u32 | 패킷 순번 |
| `channel_index` | u32 | 액터 채널 |
| `actor_net_guid` | u32 | 액터 NetGUID |
| `object_net_guid` | u32? | 서브오브젝트 NetGUID |
| `group_path` | str | `NetFieldExportGroup` 경로. RPC는 `<Class>:<Function>` |
| `handle` | u32 | 그룹 내 필드 핸들 |
| `field_name` | str? | 리플레이가 그 핸들에 선언한 이름 |
| `bit_count` | u32 | 페이로드 비트 수 |
| `raw_bits` | bytes? | 페이로드 원본 |
| `value_i64` / `value_f64` / `value_bool` / `value_str` | | 타입을 아는 경우에만 |

**`raw_bits`는 타입을 몰라도, 디코드가 실패해도 항상 남습니다.** `value_*`는 넷 중
최대 하나만 채워집니다. 나중에 포맷을 알아내도 이미 내보낸 행을 다시 파싱할 필요가
없습니다.

예외 하나: 그룹을 특정 못 해 내부 스트림 자체를 못 걷는 ClassNetCache 블록은 행이
없습니다. 그건 `validate`의 `RPC stream failed`로 카운트되고, 재해석하려면 원본
`.vrf`가 필요합니다.

배열은 평탄화돼서 `Rounds[3].Reports[1].Interactions[0].DamageDealt` 같은 이름으로
나옵니다. `LIKE 'Rounds[%].Reports[%].DamageDealt'`로 필터할 수 있습니다.

### `movement.parquet` — 캐릭터 위치 시계열

14개 컬럼: `time_ms`, `packet_id`, `character_net_guid`, `pos_x/y/z`, `yaw`, `pitch`,
`vel_x/y/z`, `timestamp`, `movement_state`, `move_type`.

**주의할 것 셋:**

- `timestamp`는 **128.0 Hz 전역 서버 틱**이고 **라운드 경계마다 리셋됩니다.** 라운드
  내 정렬에는 쓰되 전역 시간축으로 쓰지 마세요 — 그건 `time_ms`입니다.
- `movement_state`와 `move_type`은 13.01 코퍼스 전체에서 상수(0, 1)입니다. 한 빌드에서
  상수인 것과 일반적으로 상수인 것은 다르므로 와이어 그대로 내보냅니다.
- **자세 정보는 `movement_state`가 아니라 `bCrouchHeld`입니다.** 별개 필드로 이미
  `fields.parquet`에 나갑니다.

### `actors.parquet` — 액터 생성/소멸

`event`(`spawned` / `closed`), `class_path`, `archetype_path`, `spawn_x/y/z`,
`spawn_pitch/yaw/roll`. 무기·어빌리티 인스턴스의 클래스를 여기서 찾습니다.

### `net_guids.parquet` — GUID → 경로, 그리고 포함 관계

`net_guid`, `path`, `outer_net_guid`. `outer_net_guid`가 포함 사슬입니다 — 예를 들어
발사 이펙트의 `FiringState` 서브오브젝트에서 그 무기 액터로 거슬러 올라갈 때 씁니다.

### `events.parquet` — 서버가 직접 기록한 타임라인

`id`, `group`, `metadata`, `time1`, `time2`, `payload_size`, `raw_payload`.

`group`은 `characterDeath`, `characterUltimateUsed`, `roundStarted`, `spikePlanted`,
`spikeDefused`, `switchTeams` 등입니다. **페이로드 워드는 `raw_payload`로 그대로
둡니다** — 워드 개수가 그룹마다 다르고 와이어에 개수가 없으며, 7개 그룹 중 2개만 의미가
증명됐기 때문입니다. `characterDeath`의 두 워드는 killer/killed NetGUID로 확인됐습니다.

### `checkpoint_fields.parquet` — 스냅샷

`fields.parquet`와 동일한 스키마입니다. Checkpoint는 한 시점의 전체 상태 스냅샷인데
**중복이 아닙니다** — 같은 타임스탬프의 ReplayData 값과 6~11%가 불일치하고, 0.5~2%는
ReplayData가 보낸 적 없는 키입니다. 어느 쪽이 옳은지는 아직 판정 기준이 없습니다
(PROJECT_STATUS 22-I).

### `manifest.json`

ReplayInfo 전체 + 헤더 + 통계 + **리플레이가 선언한 export 그룹 전부**
(`net_field_export_groups`, 02d4d478 기준 475개). 핸들→이름 매핑이 여기 다 있습니다.

`game_specific_data`에는 `playerLoadouts` JSON이 들어 있습니다 — subject UUID별
`characterId`(요원), 스킨, 스프레이. **같은 요원을 두 명이 골랐을 때 누가 누군지
가리는 유일한 근거**입니다.

`timestamp_ticks`는 UE `FDateTime`(0001-01-01 기준 100ns)입니다. Windows FILETIME이
아닙니다 — 그렇게 읽으면 3626년이 나옵니다.

---

## 4. 라이브러리로 쓰기

필요한 층만 가져다 쓸 수 있습니다.

| 레이어 | 크레이트 | 기능 플래그 |
|---|---|---|
| 비트 리더 / UE 와이어 포맷 | `vrf-bitio` | `no_std`, `alloc` |
| 페이로드 변환 (5개 빌드) | `vrf-transform` | 없음 |
| 컨테이너 (info/header/chunk/event/checkpoint, Oodle) | `vrf-container` | `oodle` `event` `checkpoint` |
| DemoFrame 순회 | `vrf-frame` | 없음 |
| 동적 스키마 + GUID 캐시 + 체크포인트 테이블 | `vrf-schema` | `checkpoint` |
| 리플리케이션 (packet/bunch/content block/field) | `vrf-net` | `diagnostics` |
| 필드 디코더 + 중첩 배열 + 타입 오버레이 + 이펙트 | `vrf-decode` | `array` `effect` `overlay` `structs` |
| movement 디코더 | `vrf-movement` | 없음 |
| Parquet 라이터 | `vrf-export` | `parquet` + 테이블별 |
| 통합 CLI | `vrfkit` | `export` |

`vrf-bitio`는 `no_std` + 선택적 `alloc`입니다. 전 크레이트 `#![forbid(unsafe_code)]`.

ZSTD는 일부러 플래그로 빼지 않았습니다 — 모든 라이터가 그걸 고르므로, 끄면 이 크레이트가
설명할 수 없는 파일을 뱉는 빌드가 생깁니다.

---

## 5. `tools/` 레퍼런스

### 다운스트림 변환

| 스크립트 | 하는 일 |
|---|---|
| `to_valplay_bundle.py` | Parquet → NDJSON 번들(events/movement/manifest). valplay `compute_metrics.py`가 먹는 형식 |
| `equippable_table.py` | **생성 파일.** 무기 클래스 경로 → 표시명 |

```bash
python tools/to_valplay_bundle.py <export_dir> -o <bundle_dir>
python "<valplay>/pipeline/metrics/compute_metrics.py" <bundle_dir> -o metrics.json
```

### 검증 (자세한 건 6절)

`validate_corpus.py`, `validate_metrics_corpus.py`, `check_corpus_baseline.py`,
`check_export_baseline.py`, `check_decode_errors_corpus.py`,
`check_metrics_baseline.py`, `check_effect_decoder.py`, `check_ascii.py`,
`compare_combat_report.py`, `compare_rpc_params.py`, `compare_with_csharp.py`

`check_docs.py`는 이 문서 자신을 검사합니다 — 모든 `tools/` 스크립트가 여기 언급돼
있는지, 모든 크레이트가 표에 있는지, 링크가 살아 있는지, 인용된 테이블 크기와 테스트
개수가 현재 값인지. 이 저장소에서 문서 숫자는 반복적으로 낡았습니다(테스트 개수만 6번,
오버레이 테이블 크기가 1,185 → 1,187 → 1,188로 두 번 낡았고, 게임이 지운
리플레이 4개가 몇 주간 남아
있었습니다). 낡은 문장은 컴파일도 되고 테스트도 통과하므로 다른 어떤 검사도 못 잡습니다.

```bash
python tools/check_docs.py           # 테스트 스위트까지 돌려 개수 대조
python tools/check_docs.py --fast    # 개수 대조 생략
```

### 생성기 — 출력물은 절대 손으로 고치지 말 것

| 스크립트 | 생성물 |
|---|---|
| `extract_descriptors.py` | `crates/vrf-decode/src/table.rs` (오버레이 테이블 1,189 + 핸들 84) |
| `apply_type_corrections.py` | 위 파일에 검증된 정정/추가를 적용 |
| `extract_sboxes.py` | `crates/vrf-transform/src/sbox.rs` |
| `extract_golden.py` | `crates/vrf-transform/tests/data/golden_vectors.rs` |
| `extract_equippables.py` | `tools/equippable_table.py` |

**순서가 중요합니다:** `extract_descriptors.py` → `apply_type_corrections.py` →
`cargo fmt`. 정정 스크립트는 생성 직후의 한 줄 형식과 rustfmt 형식 양쪽에서 동작하지만,
`cargo fmt` 뒤에 돌리면 일부 패턴이 안 맞습니다. 그래서 이 스크립트는 자기 적용
횟수를 믿지 않고 **적용 후 최종 상태를 재검증**하고 어긋나면 실패합니다.

```bash
python tools/apply_type_corrections.py           # 적용 후 검증
python tools/apply_type_corrections.py --check   # 검증만
```

`ADDITIONS` 패스는 C# 디스크립터가 **침묵하는** 항목을 넣습니다. 현재 셋뿐입니다 —
`BaseTeamState.LoadoutValue` / `AverageLoadoutValue`(26-I, 레퍼런스가 같은
프로퍼티의 타입을 선언하고 그룹만 옮겨감)와 `BombGameState.ChosenCeremonyForRound`
(32절, 와이어 증거만). 근거 없이 넓히면 이 추가가 허용된 이유 자체가 없어집니다 —
PROJECT_STATUS 26-I와 32를 먼저 읽으세요.

### 분석 보조

`analyze_coverage.py`, `find_skips.py`

---

## 6. 검증 스위트

### 빠른 스윕 — 어떤 변경 후에도

```bash
cargo test                                        # 333 통과
cargo clippy --all-targets -- -D warnings         # 0
cargo fmt --check
python tools/check_ascii.py --check               # 113 파일, ASCII only
python tools/check_effect_decoder.py --check      # 12 케이스
python -m unittest discover -s tools/tests -p "test_*.py"   # 109 통과
python tools/check_docs.py --fast                 # 문서가 아직 이 저장소를 설명하는가
python tools/apply_type_corrections.py --check    # 28 정정 present
```

**ASCII 규칙은 스타일이 아니라 정확성 문제입니다.** Windows 콘솔이 cp949라서 포맷
문자열에 non-ASCII가 하나라도 있으면 그 지점에서 출력이 잘립니다. Rust 소스는 주석까지
ASCII입니다.

### 회귀 가드 — 비자명한 변경 후

```bash
cargo build --release -p vrfkit --features export

python tools/check_export_baseline.py --baseline tools/baselines/export_02d4d478.json
python tools/check_corpus_baseline.py --baseline tools/baselines/build_1302.json
python tools/validate_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_decode_errors_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_metrics_baseline.py
python tools/compare_combat_report.py
```

### 각 검사가 무엇을 잡는가 — 이게 핵심입니다

| 검사 | 보는 것 | 못 보는 것 | 비용 |
|---|---|---|---|
| `validate_corpus.py` | 프레이밍 (215개 전수) | 타입 오류, 의미 파손 | ~30초 |
| `check_export_baseline.py` | 23개 export 카운터 + 파일별 행·바이트 | 다른 빌드 | 1초 |
| `check_decode_errors_corpus.py` | 오버레이 타입 오류 + struct blob 실패 (215개) | 의미 파손 | ~50초 |
| `check_metrics_baseline.py` | **의미** — 라운드, 점수, K/D/A (빌드 5개) | 비-Bomb 모드 | ~65초 |
| `compare_combat_report.py` | 지표 입력값 다중집합 | 프레이밍 | 수초 |

**층이 다릅니다.** 위 넷 중 앞의 셋은 프레이밍 카운터를 읽거나 바이트를 비교하는데,
**값을 못 만들어내는 디코더는 둘 다 안 움직입니다.** 13.02가 `RoundResults` 핸들을
밀었을 때 이 검사들이 전부 green인 채로 경기 점수만 사라졌습니다.

`check_metrics_baseline.py`가 그 층을 봅니다 — `export → to_valplay_bundle →
compute_metrics`를 빌드별 보존 리플레이에 돌리고, 베이스라인이 필요 없는 불변식
다섯 개를 겁니다:

```
R1  objective.round_count > 0
R2  rounds.round_count == objective.round_count      (독립적인 두 출처)
R3  sum(team_score) == objective.round_count
R4  players > 0
R5  kills > 0 이면 damage > 0
```

**증명됐습니다:** 수정 직전 커밋(309cf05)을 빌드해 이 가드를 돌리면 13.02가 R1·R2로
실패하고 13.01은 통과합니다.

### 베이스라인 갱신

전부 `--update`를 받습니다. **DRIFT가 뜨면 각 줄을 설명한 뒤에** 쓰세요. 숫자가 신성한
게 아니라, 조용한 변경이 불가능해야 하는 게 요점입니다.

---

## 7. 지원 빌드

| 빌드 | 검증 방식 |
|---|---|
| 12.10, 12.11, 13.00 | 보존 픽스처 1개씩 + 골든 벡터 |
| 13.01 | 코퍼스 215개 전수 |
| 13.02 | 보존 리플레이 62 MB + 실측 |

13.02 실측:

```
1.vrf     (62 MB)  774,299 blocks  568,557 fields  408,591 RPCs  pass 98.919512%
f1110ea5  (59 MB)  743,110 blocks  537,865 fields  409,103 RPCs  pass 98.938381%
                   둘 다 malformed 0 / transform 실패 0 / decode errors 0
```

새 빌드를 붙이려면 `SeededTransform` 구현 하나면 됩니다 — 상수 2개와 워드 함수 3개.
자세한 건 README의 "빌드 업데이트 비용" 절.

**리플레이를 베이스라인으로 고정할 때는 `%LOCALAPPDATA%\VALORANT\Saved\Demos`를
가리키지 마세요.** 게임이 그 디렉터리를 소유하고 갈아치웁니다 — 고정해둔 리플레이 4개가
통째로 사라진 적이 있습니다. 보존본은 `%LOCALAPPDATA%\vrfkit\baseline-corpora`에
둡니다.

---

## 8. 알려진 한계

- **미타입 잔여분 ~86%** — 게임 바이너리나 UE 헤더가 필요합니다. 테이블 편집으로
  풀 수 있는 문제가 아닙니다(PROJECT_STATUS 24절).
- **`AbilitiesAndBuffsComponent`** — 리플레이가 그 클래스의 ClassNetCache 그룹을 아예
  선언하지 않습니다. 체크포인트 4,024개 전수로 확인했습니다.
- **ACS** — `PlayerScoreComponent`가 복제되지 않습니다. 계산할 근거가 없습니다.
- **`economy.per_round` (13.02)** — 팀 이코노미가 `BaseTeamState`로 옮겨갔고 값은
  디코드됩니다만, valplay의 `compute_economy`가 옛 경로를 봅니다. valplay는 수정
  대상이 아닙니다.
- **비-Bomb 게임 모드** — 코퍼스 215개 중 5개가 Swiftplay입니다(32-D).
  파싱은 되고 세리머니도 디코드되지만, 지표 파이프라인은 Bomb 전용이라
  라운드/점수/전투 리포트가 안 나옵니다. 입력이 없는 게 아니라 소비자가 없습니다.
- **ADR은 트래커보다 0.1~0.2 높습니다.** 버그가 아닙니다 — 와이어 데미지가 소수인데
  Riot API가 정수로 보고합니다. **버림을 넣어 "고치지" 마세요**(PROJECT_STATUS 27-B).
