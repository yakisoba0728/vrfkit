# vrfkit

VALORANT 리플레이(`.vrf`) 파서 및 분석 툴킷. Rust.

기존 파서들과의 차이는 목표에 있다. **리플레이에 담긴 모든 값을 내보내는 것**이
설계 전제이고, 타입을 아는 필드만 내보내는 게 아니다.

## 빠른 시작

```bash
cargo build --release -p vrfkit --features export

vrfkit inspect  <file.vrf>                          # 헤더 / 브랜치 / 청크 요약
vrfkit validate <file.vrf> [--diagnostics]          # 문법 오라클, 파일 안 씀
vrfkit export   <file.vrf> --out <dir> [--checkpoints]
```

`02d4d478`(48,215,213바이트) 기준 export 0.79초, 출력 7종:

| 파일 | 행 | 바이트 |
|---|---|---|
| `fields.parquet` | 1,246,812 | 13,742,276 |
| `movement.parquet` | 1,839,607 | 31,835,557 |
| `actors.parquet` | 3,827 | 87,281 |
| `net_guids.parquet` | 16,167 | 153,606 |
| `events.parquet` | 195 | 10,201 |
| `checkpoint_fields.parquet` | 78,748 | 191,324 |
| `manifest.json` | | 658,918 |

`checkpoint_fields.parquet` 는 `--checkpoints` 를 줘야 나오고, **켜든 끄든 나머지
다섯 테이블은 바이트 단위로 동일**합니다.

`movement.parquet` 의 `timestamp` 는 128.0 Hz 서버 틱이고 **라운드마다 리셋**됩니다 —
전역 시간축은 `time_ms` 입니다. 자세 정보는 `movement_state` 가 아니라 `bCrouchHeld`
입니다.

> **컬럼 스키마, `tools/` 스크립트, 검증 스위트, 크레이트별 사용법은
> [`docs/USAGE.md`](docs/USAGE.md) 에 있습니다.**
> 이 문서는 *왜 그렇게 만들었는지* 에 대한 것입니다.

## 상태

작업 중. 현재 검증된 범위 (`cargo test --workspace` 338 통과, `clippy -D warnings` 0, `fmt` 0):

크레이트별 개수는 `cargo test -p <crate>` 로 재세요. 아래 표에서 개수를 뺐습니다 -- 매번 낡았고, 재는 게 한 줄입니다.

| 레이어 | 크레이트 | 기능 플래그 |
|---|---|---|
| 비트 리더 / UE 와이어 포맷 | `vrf-bitio` | `no_std`, `alloc` |
| 페이로드 변환 (5개 빌드) | `vrf-transform` | 없음 (`ALL_VERSIONS` 타입이 개수를 인코딩) |
| 컨테이너 (info/header/chunk/event/checkpoint, Oodle) | `vrf-container` | `oodle` `event` `checkpoint` |
| DemoFrame 순회 | `vrf-frame` | 없음 (섹션은 커서 정렬용 바이트 범위) |
| 리플레이 동적 스키마 + GUID 캐시 + 체크포인트 테이블 | `vrf-schema` | `checkpoint` |
| 리플리케이션 (packet/bunch/content block/field) | `vrf-net` | `diagnostics` |
| 필드 디코더 + 중첩 배열 + 타입 오버레이 + 이펙트 | `vrf-decode` | `array` `effect` `overlay` `structs` |
| movement 디코더 | `vrf-movement` | 없음 (단일 프로토콜) |
| Parquet 내보내기 | `vrf-export` | `parquet` + 테이블별 |
| 통합 CLI | `vrfkit` | `export` |

필요한 것만 가져다 쓸 수 있습니다. 확인 방법:

```
cargo tree -p vrfkit --no-default-features | grep -E "arrow|parquet|zstd"
# 아무것도 안 나옵니다
```

ZSTD만 일부러 플래그로 빼지 않았습니다 -- 모든 라이터가 그걸 고르므로, 끄면 이
크레이트가 설명할 수 없는 파일을 뱉는 빌드가 생깁니다.

## 성능

`02d4d478`(48,215,213바이트) 기준:

| | 이전 | 현재 |
|---|---|---|
| `export` | 1.64초 / 201 MB | **0.79초 / 109 MB** |
| `validate` | 1.42초 / 65 MB | **0.65초 / 65 MB** |

출력은 **바이트 단위로 동일**합니다. 자세한 내역과 측정 후 기각한 최적화들은
`PROJECT_STATUS.md` 25절에 있습니다.

파일의 모든 청크 종류를 읽습니다 — ReplayData, Event, 그리고 Checkpoint. 미개봉 영역은 없습니다.

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

**추출량 — 우리가 더 많습니다.** (그룹, 필드) 쌍이 vrfkit 전용 1,450개 / 양쪽 302개 / C# 전용 71개이고, C# 전용 71개 중 49개는 명명 차이입니다(C#은 `CrouchHeld`, 우리는 와이어 이름 `bCrouchHeld`). RPC도 342,735개 대 230,893개로 48% 많습니다 — C#은 디스크립터 없는 RPC를 버립니다.

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

## Event 청크 — 서버가 쓴 타임라인

위 주장은 오랫동안 **우리 파서 자신이 유일한 증인**이었습니다. 이제 아닙니다.

`.vrf`의 Event 청크는 서버가 직접 라벨링해 기록한 이벤트 목록이고, 파일의 다른 영역에
다른 인코딩으로 들어 있으며, **기존 C# 파서는 이 청크를 열지조차 않습니다**
(`ReplayChunkDispatcher.cs:152` — `"Skipping event chunk"`). 우리는 이제 읽습니다.

```
characterDeath 132 | characterUltimateUsed 34 | roundStarted 18
spikePlanted 9 | spikeDefused 1 | switchTeams 1          (02d4d478, 195건)
```

`characterDeath` 132건은 우리가 RPC에서 뽑은 `MulticastNotifyKilledEnemy` 132건과
정확히 같고, C#의 119건 + 캐릭터 576의 13킬입니다. 페이로드의 두 워드가 killer/killed
NetGUID인데 **132/132가 순서까지 일치**했습니다(역순 매칭 0/132). 즉 "우리가 맞고 C#이
놓쳤다"는 이제 우리 주장이 아니라 서버 기록과의 대조 결과입니다.

범위를 명확히: killer/killed 짝 대조는 **리플레이 1개** 기준이고, 청크 프레이밍은
**215개 파일 43,397청크 전수**가 잔여 바이트 0으로 소비됩니다.

페이로드는 `[u32 그룹 태그][N x u32 워드][FString "EReplayEventGroup::<Name>"][f32 초]`
구조인데, `N`이 그룹마다 다르고 와이어에 개수가 없으며 7개 그룹 중 2개만 의미가
증명됐습니다. 그래서 워드를 타입 컬럼이 아니라 `raw_payload`로 내보냅니다 — 아는 것보다
많이 주장하지 않기 위해서입니다.

## 전 코퍼스 견고성 검증

`.vrf` 215개 전수를 오라클에 통과시켰습니다 (`tools/validate_corpus.py`).

```
succeeded: 215/215        failed: 0
branches : 215  ++Ares-Core+release-13.01
pass rate: min 97.487378%  median 99.323434%  max 99.682485%
totals   : 136,545,822 content blocks / 98,884,839 fields / 75,571,092 RPCs
           malformed framing 0        unattributed 1,972,018,965 bits
```

`malformed framing 0` 은 컨테이너·번치·콘텐츠 블록 프레이밍이 전 코퍼스에서 한 건도 어긋나지 않는다는 뜻입니다. 통과율이 100%가 아닌 이유는 **프레이밍이 아니라 귀속**입니다. 블록은 정확히 잘라내지만, 일부는 어느 `_ClassNetCache` 그룹의 것인지 확정할 수 없어 handle 폭을 모르고, 그래서 레코드로 풀지 못합니다.

이 수치는 한때 100%로 적혀 있었습니다. 그건 더 정확해서가 아니라 **틀렸기 때문**입니다. 당시 코드는 그룹을 못 찾은 블록을 조용히 버리면서 아무 카운터도 올리지 않았고, 오라클은 자기가 버린 데이터 위에서 만점을 보고했습니다. 그 경로를 드러내니 한 리플레이에서 14,459블록 18,831,872비트, 코퍼스 전체로 2,276,559,577비트가 나타났습니다. 이후 액터 인스턴스 이름에서 클래스 캐시 그룹을 찾아내 3억 비트가량을 회수했고, 위 숫자가 남은 양입니다.

남은 것의 경계는 명확합니다. 제한 없이 집계한 전체 실패 비트의 **97.283437%가 `AbilitiesAndBuffsComponent`** 이고, 리플레이가 그 클래스의 캐시 그룹을 아예 선언하지 않으므로 어떤 조회로도 닿을 수 없습니다. `MeleeAttackState1`~`4`와 `_Alt`는 기존 스키마 기반 resolver가 공유 `MeleeAttackStateComponent_ClassNetCache`로 이미 해석하며, 전 코퍼스 실패 블록과 비트가 모두 0입니다. 단, 미해결 ClassNetCache 블록은 내부 스트림을 걸을 수 없어 Parquet 행이나 `raw_bits`를 내보내지 못합니다. 재해석하려면 원본 `.vrf`를 보존해야 합니다.

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

고친 결과: malformed 215 → 0, 그리고 리플레이당 10개씩 총 2,150개 블록이 새로 디코드됩니다. 당시 남은 미소모 비트는 3,671개까지 줄었고, 그 4건도 뒤에 나오는 handle 최소폭 문제로 밝혀져 0이 되었습니다.

## 타입 오버레이

걸을 수 있는 필드의 원시 비트는 항상 내보내고, 타입을 아는 필드는 `value_*` 컬럼을 **추가로** 채웁니다. 타입을 몰라도, 디코드가 실패해도 그 행의 `raw_bits`는 남습니다. 그룹을 찾지 못해 내부 스트림 자체를 걸을 수 없는 ClassNetCache 블록은 예외이며, loud failure와 skipped bits만 남습니다.

오버레이 테이블은 C# 디스크립터에서 기계적으로 추출합니다(`tools/extract_descriptors.py`) — 171개 그룹 1,188개 항목. 손으로 옮기지 않는 이유는 S-box·골든 벡터와 같습니다.

`02d4d478` 기준 (`5c46851` 시점 실측):

```
Decoded OK:   369,743      Decode errors:      0
Raw/Skip:      73,984      Not in table: 511,916
No field name: 33,340      Typed:          37.4%
Effect blobs:  53,908
```

**위 여섯 줄은 이펙트 디코더가 붙은 뒤에도 하나도 움직이지 않았습니다.** 오버레이
버킷은 이펙트 패스보다 먼저 확정되므로, 이펙트로 값을 얻은 행은 여전히
`Not in table` 에 계상돼 있습니다. `Decoded OK` 에 합치면 이중 계산이 되고 베이스라인이
다른 이유로 핀한 숫자가 움직입니다. 그래서 `Effect blobs` 를 따로 보고합니다 — 이게
없으면 53,908행이 값을 얻어도 요약이 완전히 동일하게 출력됩니다.

실제 커버리지는 전체 1,246,812행 중 `value_*` 가 채워진 비율로 보세요. 이펙트 연결 전
68.8% 가 미타입이었고 지금은 **64.5%** 입니다.

**이 숫자는 자주 바뀝니다. 인용하기 전에 직접 재세요** — 여섯 개 중 네 개가 낡은 채로 방치된 적이 있습니다:

```
cargo build --release
.\target\release\vrfkit.exe export <replay.vrf> --out out\probe
```

`Typed` 는 내보낸 1,246,812행 중 `value_*` 컬럼이 채워진 비율입니다. 분모에 RPC 파라미터가 전부 들어가서 낮게 나옵니다 — `Not in table` 의 대부분이 C# 디스크립터가 없는 RPC 파라미터와, 리플레이가 선언한 475개 그룹 중 테이블에 없는 나머지입니다. 타입을 모르는 행도 `raw_bits` 는 그대로 실려 나가므로 손실이 아니라 **미해석**입니다.

**디코드 에러 0**은 215개 리플레이 전부에서 유지되며, `tools/check_decode_errors_corpus.py` 가 코퍼스 단위로 검사합니다. `vrfkit validate` 는 오버레이 카운터를 출력하지 않아서 `validate_corpus.py` 로는 잘못된 타입을 볼 수 없기 때문에 만든 가드입니다. 여기까지 오는 과정에서 와이어와 C# 선언이 어긋나는 지점 세 종류를 찾아 `tools/apply_type_corrections.py`에 근거와 함께 기록했습니다.

| 증상 | 실제 | 근거 |
|---|---|---|
| 시간 관련 `Float` 필드가 32비트 초과 소비 | wire는 `Double`(64비트) | 오차 전량이 "32비트 소비 후 32비트 잔여" |
| `215`/`216` `Int32` 필드가 3비트로 도착 | 가변폭 액터 북키핑 | C# 무기 디스크립터 주석이 "빌드마다 폭이 다르다"고 명시 |
| SmokeScreen 프로젝타일 `ReplicatedMovement` EOF | 회전이 `ByteComponents` | 같은 코드베이스의 다른 프로젝타일 4종은 전부 명시적으로 `ByteComponents` |

`byte` 폭 처리도 정정했습니다. 배열 내부 byte 프로퍼티는 유의 비트만 기록되므로 8비트 고정 읽기가 실패합니다 — C#도 `archive.BitsRemaining`만큼 읽습니다. 이걸 고치기 전에는 `AssistType`(5비트) 364행 전부가 값 없이 남았습니다.

Checkpoint는 한 시점의 전체 상태 스냅샷인데, 중복이 아닙니다 — 같은 타임스탬프의
ReplayData 값과 **6~11%가 불일치**하고 0.5~2%는 ReplayData가 보낸 적 없는 키입니다.
차이나는 키의 마지막 ReplayData 갱신은 중앙값 77초 전이라 정렬 잔차가 아닙니다.
자세한 측정은 `PROJECT_STATUS.md` 22-I, 바이트 레벨 포맷은
[`CHECKPOINT_SPEC.md`](CHECKPOINT_SPEC.md) 를 보세요.

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

- **기본 경로** — 내부 스트림을 걸을 수 있는 모든 필드를 `{group, handle, name, bit_count, raw_bits}`로
  방출한다. 타입을 모른다는 이유로 건너뛰는 분기가 없다.
- **오버레이** — `(group, handle)`에 타입이 등록돼 있으면 디코드된 값을 함께 방출한다.

나중에 어떤 필드의 포맷을 알아내도 이미 내보낸 행은 다시 파싱할 필요가 없다. 다만 미해결 ClassNetCache 블록은 행이 없으므로 원본 `.vrf`에서 다시 export해야 한다.

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

**13.02는 실측으로 확인했다.** 코퍼스 215개는 전부 13.01이라 13.02 경로는 골든 벡터로만
검증돼 있었는데, 로컬 클라이언트가 남긴 13.02 리플레이 4개를 실제로 통과시켰다.

```
1.vrf     (62 MB)  774,299 blocks  568,557 fields  408,591 RPCs  pass 98.919512%
f1110ea5  (59 MB)  743,110 blocks  537,865 fields  409,103 RPCs  pass 98.938381%
```

둘 다 **malformed framing 0, transform 실패 0, decode errors 0** 이다. 13.01과 같은
수준이며 잔여분도 같은 귀속 문제다. 기존 파이프라인이 쓰던 C# 파서는 이 빌드를 아예
거부한다.

이 자리에는 한때 다른 리플레이 4개의 수치가 적혀 있었다. 더 정확해서가 아니라
**그 파일들이 사라졌기 때문에** 바꿨다 -- `%LOCALAPPDATA%\VALORANT\Saved\Demos` 는
게임이 소유하고 갈아치우는 디렉터리다. 지금 수치는
`%LOCALAPPDATA%\vrfkit\baseline-corpora` 의 보존본에서 나온다.

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
