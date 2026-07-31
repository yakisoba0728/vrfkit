# vrfkit

VALORANT 리플레이(`.vrf`) 파서 및 분석 툴킷. Rust.

기존 파서들과의 차이는 목표에 있다. **리플레이에 담긴 모든 값을 내보내는 것**이
설계 전제이고, 타입을 아는 필드만 내보내는 게 아니다.

## 상태

작업 중. 현재 검증된 범위 (`cargo test` 190 통과, `clippy -D warnings` 0, `fmt` 0):

| 레이어 | 크레이트 | 상태 |
|---|---|---|
| 비트 리더 / UE 와이어 포맷 | `vrf-bitio` | ✅ 22 테스트 |
| 페이로드 변환 (5개 빌드) | `vrf-transform` | ✅ 골든 벡터 55/55 바이트 일치 |
| 컨테이너 (info/header/chunk, Oodle) | `vrf-container` | ✅ 32 테스트 + **실전 215/215 파일** |
| DemoFrame 순회 | `vrf-frame` | ✅ 3 테스트 |
| 리플레이 동적 스키마 + GUID 캐시 | `vrf-schema` | ✅ 34 테스트 |
| 리플리케이션 (packet/bunch/content block/field) | `vrf-net` | ✅ 28 테스트 |
| 필드 디코더 + 중첩 배열 + 타입 오버레이 | `vrf-decode` | ✅ 32 테스트 |
| movement 디코더 | `vrf-movement` | ✅ 5 테스트 |
| Parquet 내보내기 | `vrf-export` | ✅ 12 테스트, NDJSON 대비 8~25× 작음 |
| 통합 CLI | `vrfkit` | ✅ `inspect` / `validate` / `export` |

## 기존 파서와의 대조 결과

같은 리플레이(`02d4d478`)를 기존 C# 파서 출력과 대조했습니다.

**구조 — 완전 일치**

| | C# | vrfkit |
|---|---|---|
| 패킷 / 번치 | 530,401 | 530,401 |
| 액터 open / close | 2,028 / 1,799 | 2,028 / 1,799 |
| export group 경로 집합 | 475 | 475 (교집합 475, 양쪽 차집합 0) |

**movement — 사실상 비트 단위 일치.** 5만 행 조인(99.98% 성공) 기준 position 최대 오차 0.0005(float 반올림), yaw·pitch·velocity 오차 정확히 0. 행 수는 C# 1,837,220 대 우리 1,839,607 — C#이 "각 업데이트의 마지막 move만 방출"하는 한계 때문이며 우리는 중간 move 2,387개를 추가로 확보합니다.

**CombatReport 중첩 배열 — 핵심 지표 입력 전부 값 일치.** 이 구조가 K/D/A·ADR·HS%·멀티킬·관통샷의 유일한 출처라, 값 다중집합으로 대조했습니다(`tools/compare_combat_report.py`).

```
..Interactions[].AssistType                               364    364  IDENTICAL
..Interactions[].DamageDealt                              553    553  IDENTICAL
..Interactions[].DamageReceived                           553    553  IDENTICAL
..Interactions[].HitsDealt                                553    553  IDENTICAL
..Interactions[].HitsReceived                             553    553  IDENTICAL
..Interactions[].DidKill                                  414    414  IDENTICAL
..Interactions[].DealtInteractions[].Regions[].Hits       390    390  IDENTICAL
..Interactions[].DealtInteractions[].Regions[].Damage     390    390  IDENTICAL
..Interactions[].ReceivedInteractions[].Regions[].Hits    390    390  IDENTICAL
..Interactions[].ReceivedInteractions[].Regions[].Damage  390    390  IDENTICAL
```

**추출량 — 우리가 더 많습니다.** (그룹, 필드) 쌍이 vrfkit 전용 1,450개 / 양쪽 302개 / C# 전용 71개이고, C# 전용 71개 중 49개는 명명 차이입니다(C#은 `CrouchHeld`, 우리는 와이어 이름 `bCrouchHeld`). RPC도 334,641개 대 230,893개로 45% 많습니다 — C#은 디스크립터 없는 RPC를 버립니다.

**RPC 파라미터 — 값 일치, 그리고 기존 파서가 놓친 킬 13개 복구.** RPC 33만 개는 파라미터 페이로드가 통째로 raw였는데, 리플레이가 선언하는 파라미터 스키마 84개 그룹(`<Class>:<Function>` 경로)을 써서 풀었습니다. `tools/compare_rpc_params.py`로 대조:

```
MulticastNotifyDamage_Point.DamageDealt          580  580  MATCH
MulticastNotifyDamage_Point.DamageTaken          580  580  MATCH
MulticastNotifyDamage_Point.RegionalDamage       580  580  MATCH
MulticastNotifyDamage_Point.bDamageKilledTarget  580  580  MATCH
MulticastEndRound.NewRoundNumber                  17   17  MATCH
MulticastNotifyKilledEnemy.KillerCharacter       119  132  우리가 +13
```

마지막 줄이 흥미롭습니다. C#은 KillerCharacter 9종 119건, 우리는 10종 132건인데 **차이가 정확히 캐릭터 576의 13킬 하나**이고 나머지는 건수까지 전부 일치합니다.

`MulticastNotifyKilledEnemy`는 킬러의 캐릭터 액터에 호스팅되는데, 이 리플레이에서 한 플레이어의 캐릭터가 그 RPC를 전혀 복제하지 않습니다. 기존 파이프라인은 이 13킬이 타임라인에서 사라지는 걸 CombatReport 크레딧으로 되메우는 우회로 처리했습니다. 우리 파서에서는 타임라인 자체가 완전합니다.

## 전 코퍼스 견고성 검증

`.vrf` 215개 전수를 오라클에 통과시켰습니다 (`tools/validate_corpus.py`).

```
succeeded: 215/215        failed: 0
branches : 215  ++Ares-Core+release-13.01
pass rate: min 100.000000%  median 100.000000%  max 100.000000%
totals   : 136,545,822 content blocks / 98,883,979 fields / 73,742,672 RPCs
           malformed 0        skipped bits 3,671
elapsed  : 303s (1.41s per replay)
```

### 오래 걸린 버그 하나

한동안 모든 리플레이가 정확히 1개 블록과 695비트를 잃었습니다. 가설 4개를 세워 고쳐봤지만 전부 실패했고, 결국 **전수 탐색**으로 잡혔습니다 — 831비트 페이로드를 모든 시작 오프셋에서 다시 프레이밍해 "페이로드 끝에 정확히 도달하는가"로 채점했더니 오프셋 108이 통과하며 순차 짝수 GUID(64, 6, 8, 10 … 22) 10개 블록이 깨끗하게 맞았습니다. 우리는 109에서 시작했으니 **1비트 과소모**였습니다.

비트 단위 계측으로 지점을 특정했습니다.

| 서브읽기 | 비트 | 위치 |
|---|---|---|
| actor GUID `IntPacked(2)` | 8 | 0..8 |
| archetype `IntPacked(9)` | 8 | 8..16 |
| level `IntPacked(3)` | 8 | 16..24 |
| location (18비트 컴포넌트) | 63 | 24..87 |
| rotation (플래그, pitch 없음, yaw 있음, roll 없음) | 20 | 87..107 |
| scale, 없음 | 1 | 107..108 |
| **velocity, 없음** | **1** | **108..109** |

`PlayerController`는 `bReplicateMovement = false`라서 서버가 velocity를 아예 직렬화하지 않습니다 — "있지만 빈 값"이 아니라 필드가 와이어에 없습니다. 첫 번치는 `bHasPackageMapExports = false`라 경로가 아직 등록되지 않아 아키타입으로 판별할 수 없고, dynamic GUID는 0이 아닌 짝수이므로 2가 최솟값이며 리플레이가 처음 여는 dynamic 액터는 항상 리플레이 컨트롤러입니다.

고친 결과: malformed 215 → 0, skipped bits 153,096 → 3,671, 그리고 리플레이당 10개씩 총 2,150개 블록이 새로 디코드됩니다.

## 타입 오버레이

원시 비트는 항상 내보내고, 타입을 아는 필드는 `value_*` 컬럼을 **추가로** 채웁니다. 타입을 몰라도, 디코드가 실패해도 `raw_bits`는 남습니다.

오버레이 테이블은 C# 디스크립터에서 기계적으로 추출합니다(`tools/extract_descriptors.py`) — 106개 그룹 929개 필드. 손으로 옮기지 않는 이유는 S-box·골든 벡터와 같습니다.

`02d4d478` 기준:

```
Decoded OK:   240,293      Decode errors:      0
Raw/Skip:      47,143      Not in table: 108,664
No field name: 33,527      Coverage:       55.9%
```

**디코드 에러 0**은 표본 20개 리플레이에서도 유지됩니다(커버리지 50.9~59.1%, 중앙값 55.2%). 여기까지 오는 과정에서 와이어와 C# 선언이 어긋나는 지점 세 종류를 찾아 `tools/apply_type_corrections.py`에 근거와 함께 기록했습니다.

| 증상 | 실제 | 근거 |
|---|---|---|
| 시간 관련 `Float` 필드가 32비트 초과 소비 | wire는 `Double`(64비트) | 오차 전량이 "32비트 소비 후 32비트 잔여" |
| `215`/`216` `Int32` 필드가 3비트로 도착 | 가변폭 액터 북키핑 | C# 무기 디스크립터 주석이 "빌드마다 폭이 다르다"고 명시 |
| SmokeScreen 프로젝타일 `ReplicatedMovement` EOF | 회전이 `ByteComponents` | 같은 코드베이스의 다른 프로젝타일 4종은 전부 명시적으로 `ByteComponents` |

`byte` 폭 처리도 정정했습니다. 배열 내부 byte 프로퍼티는 유의 비트만 기록되므로 8비트 고정 읽기가 실패합니다 — C#도 `archive.BitsRemaining`만큼 읽습니다. 이걸 고치기 전에는 `AssistType`(5비트) 364행 전부가 값 없이 남았습니다.

```bash
cargo test
cargo clippy --all-targets -- -D warnings

# 실행
vrfkit inspect  <file.vrf>
vrfkit validate <file.vrf> [--diagnostics]
vrfkit export   <file.vrf> --out <dir>
```

`02d4d478`(48MB) 기준 export 1.8초, `fields.parquet` 10.9MB / 784,566행, `movement.parquet` 30.7MB / 1,839,607행.

## 설계

### 1. 손실이 구조적으로 불가능하게

Unreal의 프로퍼티 스트림은 **자기서술적**이다. 필드마다 핸들과 비트 길이가 값보다
먼저 나온다:

```
[1비트 체크섬] 반복 {
    handle      = IntPacked   → 0이면 종료
    payload_bits = IntPacked
    (payload_bits 만큼이 값)
}
```

타입을 몰라도 필드 경계를 정확히 걸어갈 수 있다는 뜻이다. 게다가 `handle → 이름`
매핑은 리플레이 자체가 전달하는 동적 스키마다(`NetFieldExportGroup`). 즉 **이름은
공짜고 타입만 모른다.**

그래서 디코드를 두 층으로 나눈다:

- **기본 경로** — 항상 실행. 모든 필드를 `{group, handle, name, bit_count, raw_bits}`로
  방출한다. 건너뛰는 분기가 없다.
- **오버레이** — `(group, handle)`에 타입이 등록돼 있으면 디코드된 값을 함께 방출한다.

나중에 어떤 필드의 포맷을 알아내도 이미 내보낸 데이터를 다시 파싱할 필요가 없다.

### 2. 빌드 업데이트 비용을 최소화

페이로드 변환은 게임 빌드마다 바뀐다. 하지만 릴리스 12.10부터 13.02까지 **바뀌지 않은
것**이 훨씬 많다 — PRNG와 그 승수, 시드 혼합 골격, 64→32→8→꼬리 단계 구성, 꼬리
XOR 처리, 그리고 S-box 테이블 자체까지.

빌드마다 실제로 바뀌는 것:

| | seed addend | offset | 부호 | S-box |
|---|---|---|---|---|
| release-12.10 | `0x12fd0ee5` | `0x1b` | − | 미사용 |
| release-12.11 | `0x409d36a3` | `0x23` | **+** | 미사용 |
| release-13.00 | `0x2949b6ef` | `0x11` | − | 사용 |
| release-13.01 | `0xe62fcd5c` | `0x24` | − | 미사용 |
| release-13.02 | `0x9e81a37c` | `0x04` | − | 사용 |

다섯 빌드 모두에서 **꼬리 XOR 바이트 = seed addend의 하위 바이트**다. 파생값이므로
독립 상수가 아니고, 이 관계는 테스트로 고정해 뒀다(`versions.rs`) — 앞으로 깨지면
조용히 마지막 바이트가 망가지는 대신 테스트가 실패한다.

결과적으로 새 빌드를 붙이는 일은 `SeededTransform` 구현 하나다. 상수 2개와 워드 함수
3개(`word64` / `word32` / `byte`)를 쓰면 끝이고, 나머지는 공용이다.

S-box 768바이트는 빌드 간 공유되므로 **바이너리에서 변환 함수를 찾는 시그니처**로도
쓸 수 있다.

### 3. 병렬화 지점

콘텐츠 블록 **헤더와 선언된 비트 길이는 평문**이고, 변환은 그 뒤의 페이로드에만
걸린다. 따라서 프레이밍(순차, 리플리케이션 상태기계 때문에 불가피)과 블록 디코드(완전
독립)를 분리할 수 있다. 변환은 `(bits, seed)`만으로 결정되므로 블록 단위로 병렬이다.

### 4. 출력은 Parquet

컬럼너라 반복되는 경로·이름 문자열이 딕셔너리 인코딩으로 접히고, zstd가 잘 먹으며,
`pyarrow` / `polars` / `pandas` / `duckdb`에서 바로 읽힌다. NDJSON은 읽는 쪽이
병목이다 — 280만 행 이동 스트림에서 JSON 파싱이 처리 시간의 84%를 먹는다는 실측이
있다.

## 생성 파일 갱신

`crates/vrf-transform/src/sbox.rs`와 `crates/vrf-transform/tests/data/golden_vectors.rs`는
생성 파일이다. 손으로 고치지 말 것. 갱신하려면 upstream 체크아웃이 필요하다:

```bash
python tools/extract_sboxes.py <path>/ValorantSeededTransformHelpers.cs \
    crates/vrf-transform/src/sbox.rs
python tools/extract_golden.py <path>/ValorantSeededTransformTests.cs \
    crates/vrf-transform/tests/data/golden_vectors.rs
```

두 생성기 모두 무결성 검사를 내장한다 — S-box는 0..255 순열인지, 골든 벡터는 hex 길이가
비트 수와 맞는지 확인하고 아니면 생성을 거부한다.

## 라이선스

MIT. 파생 관계와 원저작자 표시는 [`NOTICE.md`](NOTICE.md) 참고.

Riot Games와 무관한 커뮤니티 도구다.
