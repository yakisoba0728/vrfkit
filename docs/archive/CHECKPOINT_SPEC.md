# `.vrf` Checkpoint 청크 -- 포맷 명세

VALORANT `.vrf` 리플레이의 Checkpoint 청크 포맷을 바이트 단위로 규정한다. 대상은 청크
프레이밍, 감압 후 아카이브 프로로그, `GuidCacheEntry`, `NetFieldExportGroup`, `DemoFrame`,
그리고 첫 파서 시도를 꺾은 바이트 단위의 함정들이다.

> **상태: 구현 완료.** 이 문서는 읽기 전용 조사로 시작했으나 현재는 출하된 코드의 기준
> 문서다. 파서는 `vrf-container`의 `checkpoint` 모듈(청크 헤더, 감압)과 `vrf-schema`의 두
> 테이블에 있으며, `vrfkit export --checkpoints` 뒤에서 실행된다. 무엇이 구현되었는지는
> PROJECT_STATUS.md 23절, 구현을 정당화한 측정은 22-I에 있다.
>
> 코드가 이미 존재하는 지금도 읽을 가치가 있는 부분이 둘 있다. 9절은 기각된 가설들을,
> 10절은 여전히 미지수인 항목들을 기록하며, 둘 다 코드에는 반영되지 않았다.
>
> 조사 일자: 2026-08-04. 아래가 인용하는 probe는 세션 스크래치패드에 있던 독립 cargo
> 프로젝트로 현재는 사라졌다. 프로덕션 파서는 이 문서의 모든 수치 -- 체크포인트 4,024건,
> guid 엔트리 17,186,645개, 그룹 레코드 1,955,988개, 익스포트 슬롯 11,529,869개, 평문
> 2,967,025,362바이트, 에러 0건 -- 를 재현하며, 이 교차 검증이 두 구현을 서로 맞춘 방법이다.

## 목차

- 0. [헤드라인](#0-헤드라인)
- 1. [청크 단위 레이아웃](#1-청크-단위-레이아웃)
- 2. [감압 후 아카이브 레이아웃](#2-감압-후-아카이브-레이아웃)
  - 2.1 [20바이트 프로로그](#21-20바이트-프로로그)
  - 2.2 [GuidCacheEntry](#22-guidcacheentry)
  - 2.3 [NetFieldExportGroup 레코드](#23-netfieldexportgroup-레코드)
  - 2.4 [DemoFrame](#24-demoframe)
- 3. [질문 2 -- 169,586과 169,593 사이의 "7바이트"](#3-질문-2--169586과-169593-사이의-7바이트)
- 4. [질문 3 -- ExportData 가 "unknown path name index 0" 로 실패한 이유](#4-질문-3--exportdata-가-unknown-path-name-index-0-로-실패한-이유)
- 5. [질문 4 -- ReplayData 가 선언하지 않는 익스포트 그룹이 체크포인트에 있는가](#5-질문-4--replaydata-가-선언하지-않는-익스포트-그룹이-체크포인트에-있는가)
  - 5.1 [`_ClassNetCache` 답: 없다](#51-_classnetcache-답-없다)
  - 5.2 [체크포인트가 실제로 추가하는 것](#52-체크포인트가-실제로-추가하는-것)
- 6. [질문 5 -- 프레임의 내용과 ReplayData 와의 관계](#6-질문-5--프레임의-내용과-replaydata-와의-관계)
- 7. [질문 6 -- 가장 작은 정확 구현](#7-질문-6--가장-작은-정확-구현)
- 8. [주장별 검증 방법](#8-주장별-검증-방법)
- 9. [기각된 가설과 정정](#9-기각된-가설과-정정)
- 10. [미해결 / 미지수](#10-미해결--미지수)

> **주요 함정 (load-bearing).** 다음이 첫 파서를 실패시킨 지점이며, 본문 각 절에서 상술한다.
> 1. `NumNetFieldExports` 는 **`IntPacked`** 이지 `u32` 가 아니다 (같은 섹션의 머리 `u32` 인 `NumNetFieldExportGroups` 와 인코딩이 다르다). -- §2.3
> 2. guid 엔트리의 필드 순서는 **`(NetGUID, OuterGUID)`** 이다 (역순가 아니다). -- §2.2
> 3. guid 엔트리의 경로 판별 바이트(`PathIsString`)의 극성은 `FName` 의 `bHardcoded` 와 **반대**다. 여기서 `1` = 문자열, `0` = 하드코드 인덱스. `read_fname` 을 재사용하지 말 것. -- §2.2
> 4. DemoFrame 은 `map_end == w0 + 8` 에서 시작한다 (`w0 + 7` 이 아니다). -- §2.1, §3
> 5. `AbilitiesAndBuffsComponent_ClassNetCache` (및 모든 유사 표기)는 **4,024개 체크포인트 전체에서 부재**다. -- §5.1

---

## 0. 헤드라인

**체크포인트 청크 전체가 바이트 정확으로 해독되었으며, 설명되지 않은 바이트는 0바이트다.**
전 코퍼스 -- 파일 215개, 체크포인트 4,024건, 위반 0건 -- 에서 검증되었다.

**질문 4의 답: 아니다.**
ReplayData 가 선언하지 않은 `_ClassNetCache` 익스포트 그룹은 어떤 체크포인트에도 존재하지
않는다. `AbilitiesAndBuffsComponent_ClassNetCache` (및 모든 유사 표기)는 **215개 코퍼스
파일의 4,024개 체크포인트 전체에서 부재**다. 체크포인트는 `AbilitiesAndBuffsComponent` 를
언락하지 않는다. 상세와 정확한 측정은 §5.

체크포인트가 ReplayData 에 없는 것으로 *포함하는* 내용: 리플레이당 **46~51개의 추가
익스포트 그룹 *경로*** (모두 일반 RepLayout 그룹, `_ClassNetCache` 는 없고, 거의 전부 선언된
필드 슬롯이 **0**), 더해 완전한 **NetGUID -> 경로 테이블** (1,000~5,000 엔트리)과 **체크포인트당
풀스테이트 DemoFrame 한 개**.

---

## 1. 청크 단위 레이아웃

`ChunkType == 2` (`ChunkType::Checkpoint`). 페이로드:

| 오프셋 | 타입 | 필드 | 검증값 |
|---|---|---|---|
| 0 | `FString` | `Id` | `"checkpoint0"`, `"checkpoint1"`, ... (0-based, 청크 순서와 일치) |
| ... | `FString` | `Group` | 항상 `"checkpoint"` |
| ... | `FString` | `Metadata` | `"1"`, `"2"`, ... (1-based 카운터) |
| ... | `u32` | `Time1` | ms |
| ... | `u32` | `Time2` | **항상 `Time1` 과 동일** |
| ... | `i32` | `SizeInBytes` | 뒤따르는 아카이브의 바이트 수 |
| ... | `[u8; SizeInBytes]` | archive | Oodle 프레이밍 (아래) |

이 세 `FString` 은 **UTF-8** (양수 길이)이며, 아카이브 내부의 모든 `FString` (§2, UTF-16LE)과
다르다. 헤더 크기가 달라지는 원인은 이것이다:

| 청크 | `Id` | `Metadata` | 헤더 바이트 |
|---|---|---|---|
| `checkpoint0`..`checkpoint8` | 11 ch -> 4+12 | `"1"`..`"9"` -> 4+2 | 16+15+6+12 = **49** |
| `checkpoint9` | 11 ch -> 4+12 | `"10"` -> 4+3 | 16+15+7+12 = **50** |
| `checkpoint10`+ | 12 ch -> 4+13 | `"11"`+ -> 4+3 | 17+15+7+12 = **51** |

메인 세션이 관찰한 49/50/51 변동은 문자열 길이로 완전히 설명되며, 숨겨진 필드는 없다.

아카이브 프레이밍 (ReplayData 와 동일):

```
[i32 decompressed_size][i32 compressed_size][oodle bytes]
```

`compressed_size + 8 == SizeInBytes` 이고 `decompressed_size == 평문 길이`:
**4,024개 코퍼스 체크포인트 전체에서 검증, 위반 0건.**

> **정정.** 이전 기록에서 `compressed_size + 8 == SizeInBytes` 를 참조 리플레이에서만 검증했다고
> 적었다. 현재는 코퍼스 전체에서 성립한다.

---

## 2. 감압 후 아카이브 레이아웃

```
+--------------------------------------------------------------+ 0
| u32  DemoFrameOffset     (see §2.1 -- frame starts at this+8)  |
| u32  0                                                        |
| u32  0                                                        |
| u32  0                                                        |
| u32  NumGuidCacheEntries                                      | 16
+--------------------------------------------------------------+ 20
| GuidCacheEntry x NumGuidCacheEntries        (§2.2)            |
+--------------------------------------------------------------+ gc_end
| u32  NumNetFieldExportGroups                                  |
| NetFieldExportGroup x NumNetFieldExportGroups   (§2.3)        |
+--------------------------------------------------------------+ map_end
| exactly ONE DemoFrame, byte-identical grammar to ReplayData   |
+--------------------------------------------------------------+ end of buffer
```

모든 읽기는 **바이트 정렬** (`FBinaryArchive` 의미론)이며, ReplayData 의 DemoFrame 과 동일하다.

### 2.1 20바이트 프로로그

- `u32 @0` -- 이하 `w0`. **`map_end == w0 + 8` 이 4,024개 코퍼스 체크포인트 전체에서 성립, 예외 0건.**
  즉 `w0` 는 DemoFrame 의 오프셋이되 *아카이브의 0바이트가 아니라 8바이트에서 잰 값*이다.
- `u32 @4`, `u32 @8`, `u32 @12` -- **4,024개 체크포인트 전체에서 모두 0** (명시적으로 확인).
- `u32 @16` -- guid 캐시 엔트리 수. 정확한 폐쇄로 확인: 이 수만큼 엔트리를 파싱하면 커서가
  타당한 `NumNetFieldExportGroups` 위에 놓이고, 이것이 다시 `w0 + 8` 에서 닫힌다.

**검증됨**: `w0 + 8 == map_end == DemoFrame 시작`; 4/8/12번째 워드는 0.
**추론 (미검증)**: 바이트 0..8 은 단일 `int64` (오프셋), 바이트 8..16 은 두 번째 `int64`
(VALORANT 에서 항상 0인, 삭제된 시작업 액터 / 델타 체크포인트 필드로 추정)일 가능성.
상위 워드가 0이기 때문에 두 해석을 구분할 수 없다.

> **구현자 실용 규칙: `w0` 를 전혀 사용하지 말 것.** 두 테이블을 파싱하면 프레임은
> 익스포트 그룹 맵이 끝나는 지점에서 정확히 시작한다. `w0` 는 일관성 단언용으로만 쓴다.

### 2.2 GuidCacheEntry

질문 1에 대한 답.

```
NetGUID     : IntPacked
OuterGUID   : IntPacked
PathIsString: u8            -- 0 or 1 only
  if 1:  PathName  : FString       (UTF-16LE, always negative length; NO trailing i32 number)
  if 0:  NameIndex : IntPacked     (hardcoded-name-table index)
Flags       : u8            -- 0x00 or 0x03 only
```

> **함정 (극성).** `read_fname` 에서 선행 `1` 바이트는 *하드코드 인덱스*를 의미하지만, 여기서는
> 선행 `1` 이 *문자열*을 의미한다. 두 바이트는 같은 필드가 아니며; guid 테이블에 `read_fname`
> 를 재사용하지 말 것.

> **함정 (필드 순서).** `IntPacked` 쌍은 `(NetGUID, OuterGUID)` 순서다. 역순이나 단일 2바이트
> packed 값이 아니다. `03 0a` 를 하나의 `IntPacked` = 641 로 읽으면 외부 체인이 해석 불가능하다.

실전 예, `02d4d478-...` 의 체크포인트 0, 감압 후 오프셋 (단위: 바이트):

| 오프셋 | 바이트 | 디코드 |
|---|---|---|
| 20 | `0e` | `IntPacked` -> NetGUID **7** |
| 21 | `00` | `IntPacked` -> OuterGUID **0** |
| 22 | `01` | PathIsString = 1 |
| 23 | `e7 ff ff ff` | FString 길이 **-25** -> 25 UTF-16 유닛 |
| 27..76 | ... | `"/Game/Maps/Ascent/Ascent"` |
| 77 | `03` | Flags = 0x03 -- **엔트리 0의 끝** |
| 78 | `0a` | NetGUID **5** |
| 79 | `0e` | OuterGUID **7** <- 바로 위 패키지 |
| 80 | `01` | PathIsString = 1 |
| 81 | `f9 ff ff ff` | -7 -> `"Ascent"` |
| ... | | Flags, 이어서 다음 엔트리 |

필드 순서가 `(NetGUID, OuterGUID)` 임을 보이는 체인 증거: `"Ascent"` (guid 5)의 outer 는 7 =
패키지 `/Game/Maps/Ascent/Ascent`; `"PersistentLevel"` (guid 3)의 outer 는 5 = 월드 `Ascent`;
`"Default__BaseReplayController_C"` (guid 9)의 outer 는 11 = 패키지
`/Game/Characters/_Core/BaseReplayController`. 반대로 읽으면 (641/385/1409 등) 이런 체인이
전혀 없는 무의미한 값이 나온다.

측정된 필드값 분포 -- **관측 범위가 다르므로 동일한 주장이 아님에 주의**:

| 필드 | 관측값 | 범위 |
|---|---|---|
| `PathIsString` | 오직 `0` 과 `1`, **그 외 바이트는 없음** | **코퍼스 전체: 파일 215개, 엔트리 17,186,645개.** 파서가 세 번째 값에서 하드 에러, `verify` 위반 0건 |
| `PathName` FString 길이 | **항상 음수 (UTF-16LE); 양수/UTF-8 길이 0건** | **코퍼스 전체: guid 경로 + 그룹 경로 + 필드 이름 합산 FString 25,038,008개** |
| `PathIsString` 분할 | `1` x1,134,611 (75.7%), `0` x365,091 (24.3%) | 파일 20개 / 엔트리 1,499,702개 |
| `Flags` | `0x00` x998,076, `0x03` x501,626 -- 세 번째 값 미관측 | **파일 20개만.** 파서가 이 바이트를 검증하지 않으므로 코퍼스 다른 곳의 세 번째 값은 잡히지 않음 |
| `OuterGUID == 0` | 498,762 엔트리 (33%) | 파일 20개 |
| GUID 패리티 | 홀수 (정적) 1,120,038; **짝수 (동적) 379,664** | 파일 20개 |

참고:

- 이 테이블은 **정적 GUID 전용이 아니다**: 엔트리의 25% 가 짝수 (동적) GUID 를 가진다.
  `is_dynamic()` 으로 필터링하는 구현은 테이블의 4분의 1을 떨어뜨린다.
- `NameIndex` (`PathIsString == 0` 가지)는 **경로가 아니라 이름-테이블 인덱스**다. 증거:
  체크포인트 0 은 이런 엔트리 157개를 가지지만 **고유값은 25개**뿐이며, 값이 형제 서브오브젝트
  간에 고정 패턴으로 반복된다 -- 예: `(guid 38, outer 34, 18)`, `(40, 34, 155)`,
  `(42, 34, 156)`, 이어서 `(48, 44, 18)`, `(50, 44, 155)`, `(52, 44, 156)`, .... 체크포인트 17 은
  이런 엔트리 1,204개에 고유값 159개. 이것은 `vrf-schema` 의 `read_fname` 이 이미 처리하는
  동일한 "하드코드 FName" 메커니즘이다 (`crates/vrf-schema/src/reader.rs:57-67`, 해당 인덱스를
  십진수로 표기). **검증됨**: 값 분포와 재사용 패턴. **추론**: 이 테이블이 엔진/게임 글로벌
  이름 테이블이라는 점 (내용이 리플레이에 없으므로 파일만으로 인덱스를 텍스트로 복원 불가).
- `Flags` -- UE 의 `bNoLoad | (bIgnoreWhenMissing << 1)` (0x03 = 둘 다 설정)로 **추론**.
  두-값 분포만 검증되었다.
- **미지수**: 이 레코드 어디에도 `NetworkChecksum` 이 존재하는가. 존재하지 않는다: 레코드는
  그것을 넣을 여지 없이 정확히 닫힌다. UE 의 `SerializeGuidCache` 가 하나를 쓴다 해도 이
  빌드는 쓰지 않는다.

### 2.3 NetFieldExportGroup 레코드

```
u32 NumNetFieldExportGroups
repeat NumNetFieldExportGroups times:
   PathName          : FString    (UTF-16LE)
   PathNameIndex     : IntPacked
   NumNetFieldExports: IntPacked          <-- IntPacked, NOT u32
   repeat NumNetFieldExports times (slot index i = 0..N):
       bExported : u8      -- 0 or 1
       if bExported:
           Handle             : IntPacked   (always == i; 11,529,869 exported slots corpus-wide, 0 violations)
           CompatibleChecksum : u32
           ExportName         : FName
                                  u8 bHardcoded
                                  if 1: IntPacked NameIndex
                                  if 0: FString Name (UTF-16LE); i32 Number
```

> **함정.** `NumNetFieldExports` 는 **`IntPacked`** 이다 (`u32` 가 아니다). 같은 섹션의 머리에 있는
> `NumNetFieldExportGroups` 는 평범한 `u32` 다. 두 카운트가 같은 섹션에서 다른 인코딩을 쓴다.
> `0x2a` 를 `u32` 로 읽으면 42 (잘못된 값), `IntPacked` (`>> 1`)로 읽으면 21 (참값) 이다.

실전 예 (체크포인트 0, 그룹 0):

| 오프셋 | 바이트 | 디코드 |
|---|---|---|
| 139140 | `60 00 00 00` | `NumNetFieldExportGroups` = **96** (u32) |
| 139144 | `bd ff ff ff` | -67 -> 66-문자 경로 |
| 139148..139281 | | `"/Game/Characters/_Core/BaseReplayController.BaseReplayController_C"` |
| 139282 | `02` | `PathNameIndex` = 1 |
| 139283 | `2a` | `NumNetFieldExports` = **21** (IntPacked; `0x2a >> 1`) |
| 139284..286 | `00 00 00` | 슬롯 0,1,2 미익스포트 |
| 139287 | `01` | 슬롯 3 익스포트 |
| 139288 | `06` | handle = **3** (== 슬롯) |
| 139289 | `85 51 f9 f4` | checksum `0xf4f95185` |
| 139293 | `01` | FName 하드코드 |
| 139294 | `b1 02` | name index 216 |
| 139296..303 | `00`x8 | 슬롯 4..11 |
| 139304 | `01 18 ...` | 슬롯 12, handle 12, name index 215 |
| ... | | 슬롯 14 -> FString `"PlayerState"` + `i32 0`; 슬롯 18 -> `"SpawnLocation"` |
| 139401 | | 다음 그룹의 FString 길이 -- 레코드가 정확히 21 슬롯에서 닫힘 |

`NumNetFieldExports` 를 `u32` 로 읽으면 첫 그룹이 42 슬롯을 주장하고 오프셋 139,401 의 다음
그룹 FString 안으로 넘쳐난다; `IntPacked` 으로 읽으면 21 을 주장하고 정확히 그 지점에서 닫힌다.

### 2.4 DemoFrame

ReplayData 의 DemoFrame 문법과 바이트 동일. `vrf_frame::iter_demo_frames` 가 **수정 없이** 파싱한다.

4,024개 코퍼스 체크포인트 전체에서 측정:

| 속성 | 결과 |
|---|---|
| 체크포인트 아카이브당 DemoFrame | **정확히 1** (4,024 프레임 / 4,024 체크포인트, 예외 0건) |
| `timeSeconds x 1000` vs 청크 `Time1` | **모든** 프레임에서 1 ms 이내로 일치 |
| 0이 아닌 net-field-export 수를 가진 프레임 | **0** |
| 0이 아닌 export-GUID 수를 가진 프레임 | 4,024 중 3,809 |
| 총 패킷 | 904,891 |

프레임은 net-field-export 선언을 **하나도** 담지 않는다. 프레임의 스키마 전체가 같은 아카이브
앞에 오는 익스포트 그룹 맵에서 온다.

---

## 3. 질문 2 -- 169,586과 169,593 사이의 "7바이트"

**전제가 틀렸다; 그런 바이트는 없고 169,593 은 프레임 시작이 아니다.**

`w0 = 169,586` 은 섹션 경계가 아니라 -- *익스포트 그룹 맵의 마지막 필드 이름 UTF-16 널
터미네이터 내부*에 놓인다. 맵의 마지막 바이트:

```
0x29670  74 00 00 00 | 00 00 00 00 | 00 00 | 00 00 00 00  a2 20 ...
         ^t  ^hi ^-- NUL --^  ^ i32 Number = 0 ^  ^slots^  ^-- DemoFrame --
         169584                169588          169592     169594
```

- 169,584-169,587: 필드 이름의 마지막 두 UTF-16 유닛 (`...Event` + NUL).
- 169,588-169,591: FName 의 `i32 Number` = 0.
- 169,592, 169,593: `bExported = 0` 슬롯 바이트 두 개, 마지막 그룹을 닫음.
- **169,594**: `i32 currentLevelIndex = 0`, 이어서 `f32 timeSeconds = 0.047638...` (청크 `Time1` = 47).

> **함정.** 올바른 규칙은 `frame_start = w0 + 8 = map_end` 이다. "7바이트" 는 `w0 + 7` 을
> 추측한 결과물이다.

---

## 4. 질문 3 -- ExportData 가 "unknown path name index 0" 로 실패한 이유

**답: 가설 (a) 도 (b) 도 아니다. 프레임 시작 오프셋이 1바이트 앞이었다.**

체크포인트 0 에서 **새로 빈** `NetGuidCache` 로 `w0 + delta` 시작점을 sweep 한 `iter_demo_frames`
직접 측정:

| 시작 | 결과 |
|---|---|
| `w0+0` (169586) | Err -- packed integer 가 5바이트 내에서 종료되지 않음 |
| `w0+1` | Err -- 아카이브 끝: 50331648 비트 필요 |
| `w0+2` | Err -- 길이 4014880 무효 |
| `w0+3` | Err -- 비트 레벨 읽기 실패 |
| `w0+4` | Err -- **unknown path name index 16** |
| `w0+5` | Err -- **unknown path name index 3873** |
| `w0+6` | Err -- **unknown path name index 0** |
| `w0+7` | Err -- **unknown path name index 0**  <- 메인 세션이 보고한 정확한 에러 |
| **`w0+8`** (169594) | **`Ok(64)` -- 64 패킷, 41,114 바이트** |
| `w0+9` | Err -- export GUID 페이로드 크기가 음수: -25 |
| `w0+10` | Err -- packed integer 가 종료되지 않음 |

- **가설 (a) -- "캐시를 ReplayData 에서 체크포인트 시각까지 미리 채워야 한다": 기각.** 새로 빈
  캐시로 동작한다. 코퍼스 전체에서 4,024개 중 **0개** 의 체크포인트 프레임이 net-field-export
  레코드를 담으므로, 시드가 충족할 것이 없다.
- **가설 (b) -- "체크포인트는 변형 프레임 인코딩을 쓴다": 기각.** 스톡 `flags` 의 스톡
  `iter_demo_frames` 가 수정 없이 모든 체크포인트 프레임을 파싱한다.
- **가설 (c) -- 정렬 어긋남: 확정.** `UnknownPathIndex { index: 0 }` 는 단지 잘못 정렬된 바이트를
  넘겨받았을 때 `read_net_field_exports` (`crates/vrf-schema/src/reader.rs:76-97`) 가 내보내는
  것: `path_name_index = 0` 을 읽고, `is_exported != 1` 을 보며, 그룹 0 을 찾지 못한다. 이것은
  스키마 가용성의 시그니처가 아니라 정렬 어긋남의 시그니처다.

> **"시드가 필요 없다"로 읽지 말 것.** 빈 캐시는 프레임 **프레이밍** 에는 충분하다 --
  `iter_demo_frames` 가 패킷에 도달하는 건 프레임이 익스포트를 선언하지 않기 때문이다. 하지만
  체크포인트 자체의 익스포트 그룹 맵 (§2.3) 과 guid 테이블 (§2.2) 로부터 `NetGuidCache` 를 시드하는
  것은 **하류 작업에 필수**다: `ReplicationReader` 가 그룹 경로와 필드 이름을 그 캐시에서
  해석하며, 프레임이 익스포트 레코드를 0개 담으므로 맵이 아카이브 안의 **유일한** 스키마 출처다.
  §6 의 측정은 시드된 캐시로 생성되었다; 시드 없는 실행은 같은 패킷을 프레이밍하고 아무 이름도
  붙이지 않는다.

---

## 5. 질문 4 -- ReplayData 가 선언하지 않는 익스포트 그룹이 체크포인트에 있는가

### 5.1 `_ClassNetCache` 답: 없다

| 측정 | 결과 |
|---|---|
| 스캔한 체크포인트 | **215개 코퍼스 파일 전체의 4,024건** |
| 파싱된 익스포트 그룹 레코드 | 1,955,988 |
| `AbilitiesAndBuffs` 포함 그룹 경로 | **0** |
| `Buff` 포함 그룹 경로 | **0** (참조 리플레이, RD 와 CP 모두) |
| `_ClassNetCache` 그룹, 참조 리플레이 | ReplayData **147**, 체크포인트 **147**, 체크포인트 전용 **0** |
| `_ClassNetCache` 체크포인트 전용, 표본 4파일 | **0, 0, 0, 0** |

> **부정 결과 (load-bearing).** `AbilitiesAndBuffsComponent` 는 이 파일들에 **존재**하긴 한다 --
> 단 *NetGUID 오브젝트 경로* (서브오브젝트 인스턴스 이름)로만, 그것도 ReplayData 와 체크포인트
> *양쪽* guid 테이블에. 어느 쪽에서도 익스포트 그룹 선언은 아니다. 이것이 중요한 영역-분류
> 구별이다: GUID 테이블의 히트는 아무것도 언락하지 않는다. guid 테이블은 NetGUID->오브젝트경로
> 이름공간이지 `NetFieldExportGroup` 이름공간이 아니며, vrfkit 은 이미 그 사상을 ReplayData 의
> export-GUID 번치에서 가지고 있다.

**체크포인트는 `AbilitiesAndBuffsComponent` 를 언락하지 않는다.** 97.3% 의 귀속불능 비트 문제는
이 작업으로 변하지 않는다. 서버는 그 클래스의 ClassNetCache 레이아웃을 파일 어디에서도
보내지 않는다.

### 5.2 체크포인트가 실제로 추가하는 것

파일당, 모든 체크포인트의 합집합 vs. ReplayData 스트림 전체:

| 파일 | RD 그룹 | CP 그룹 | CP 전용 | RD 전용 | CP 전용 `_ClassNetCache` |
|---|---|---|---|---|---|
| `02d4d478-...` (참조) | 475 | 522 | **48** | 1 | 0 |
| `03c60af4-...` | 418 | 466 | **51** | 3 | 0 |
| `3e835083-...` | 467 | 510 | **46** | 3 | 0 |
| `b261cc25-...` | 499 | 543 | **47** | 3 | 0 |

참조 리플레이의 48개 체크포인트 전용 그룹은 모두 일반 RepLayout 클래스다. 예:
`/Script/ShooterGame.DamageableComponent`, `.ForceModuleManagerComponent`,
`.BlindManagerComponent`, `.UltPointsComponent`, `.DownedComponent`, `...Modifier_C`
블루프린트 11개, `/Script/Engine.AnimInstanceReplicationComponent`.

**그러나 이들은 거의 전부 빈 선언이다.** 필드 이름 커버리지, 참조 리플레이:

| 측정 | 값 |
|---|---|
| ReplayData `(group, handle) -> name` 쌍 | 3,226 |
| 체크포인트 `(group, handle) -> name` 쌍 | 3,223 |
| 체크포인트에만 있는 쌍 | **15** |
| ReplayData 에만 있는 쌍 | 18 |
| 양쪽이 불일치하는 쌍 | 379 -- **전부 표면적** (`#216` vs `216`: probe 가 하드코드 이름 인덱스에 접두; `vrf-schema` 는 그러지 않음) |
| RD 와 CP 간 선언 길이가 다른 그룹 | **0** |

즉 체크포인트의 익스포트 그룹 맵은, 실용 목적상 **ReplayData 가 이미 전달하는 같은 스키마**다.
참조 리플레이에서 3,226 중 15개의 새 이름있는 핸들 (0.5%)만 기여한다. 추가 46~51개의
*경로* 는 선언된 용량을 가지지만 익스포트된 필드 이름은 없다.

---

## 6. 질문 5 -- 프레임의 내용과 ReplayData 와의 관계

참조 리플레이, 체크포인트 프레임 패킷 위에서 `vrf_net::ReplicationReader` 로 측정
(캐시는 체크포인트 자체의 두 테이블에서 시드):

```
ReplayData (whole match): packets=530,401  bytes=112,887,672  actor opens=2,028
                          rep-layout blocks=258,882  CNC blocks=349,119  fields=429,627

cp0  t=47       1 frame  64 packets   41,114 B  opens= 64  rep=  300  cnc= 0  fields= 2,010
cp1  t=91,927   1 frame 187 packets   82,471 B  opens=159  rep=1,035  cnc= 8  fields= 4,730
cp5  t=587,447  1 frame 203 packets  118,570 B  opens=158  rep=  901  cnc=10  fields= 4,192
cp17 t=1,697,092 1 frame 261 packets 201,462 B  opens=169  rep=  974  cnc=10  fields= 4,214
```

- 모든 체크포인트 프레임은 **풀스테이트 스냅샷**이다: 그 순간 살아있는 모든 액터 (매치 중반
  ~160 액터) 의 액터 채널을 다시 열고 그 액터의 완전한 RepLayout 프로퍼티 상태를 재전송한다.
  아카이브가 매치 시간에 따라 단조 증가하는 이유가 이것이다.
- **액터 *집합*은 ReplayData 의 부분집합이다.** cp0 의 64개 액터 GUID 전체, cp5 의 158개 전체가
  ReplayData 스트림의 액터 오픈으로도 나타난다 (양쪽 스폿체크 모두 100% 겹침).
  **이것은 집합 소속만이다.** 체크포인트가 담는 프로퍼티 *값* 이 같은 타임스탬프에서
  ReplayData 가 담은 것과 일치하는지는 **측정되지 않았다** (§10). "중복" 을 확립된 것으로
  취급하지 말 것 -- 기대일 뿐, 발견이 아니며, §7 의 5번 항 배출 결정은 이에 의존한다.
- 참조 리플레이에서 ReplayData 대비 볼륨: **패킷 바이트의 2.2%** (2,477,136 /
  112,887,672), **디코드된 필드의 18.1%** (77,812 / 429,627). 필드 비율이 바이트 비율보다 훨씬
  높은 이유는 스냅샷 패킷이 프로퍼티 데이터로 조밀하고 이동/RPC 트래픽을 거의 담지 않기
  때문이다.
- 내용의 거의 전부는 RepLayout 이다. ClassNetCache 블록은 체크포인트당 0-10 (vs. ReplayData
  349,119). **주의: probe 싱크가 `function_count = 0` 을 반환하므로 RPC 를 디코드하지 않는다;
  블록 수는 신뢰, RPC 수는 미측정.**
- 진정으로 비-중복인 내용은 스냅샷의 *완전성*이다: ReplayData 의 파서가 읽는 첫 번째 청크
  이전에 한 번만 복제되고 이후 다시는 복제되지 않은 프로퍼티는 모든 체크포인트에 존재한다.
  그 가치가 있는지는 vrfkit 이 스트림을 처음부터 읽고 있는지 (그렇다) 에 달려 있으므로
  기대값은 낮다.
- **미지수 / 미측정**: 체크포인트의 개별 `(actor, handle)` 값이 같은 타임스탬프에서
  ReplayData 가 담은 것과 다른지 여부.

---

## 7. 질문 6 -- 가장 작은 정확 구현

### 수정 없이 재사용 (편집 불필요)

| 컴포넌트 | 그대로 동작하는 이유 |
|---|---|
| `vrf_container::ChunkIterator` | 이미 `ChunkType::Checkpoint` 를 yield |
| `vrf_container::decompress_replay_data` | 아카이브 본문이 동일한 Oodle 프레이밍 |
| `vrf_frame::iter_demo_frames` | 체크포인트 DemoFrame 을 **수정 0** 으로, 스톡 `flags` 로 파싱 |
| `vrf_schema::NetGuidCache` / `NetFieldExportGroup` / `NetFieldExport` | 체크포인트 테이블이 기존 public setter (`add_export_group`, `set_field_on_group`, `set_net_guid_path`) 로 정확히 이 구조체들을 채운다 |
| `vrf_net::ReplicationReader` | 내보낸 패킷을 수정 없이 소비 |
| `vrf_bitio::BitReader` | `read_int_packed`, `read_fstring` (음수/UTF-16 처리), `read_u8/u32/i32` 가 필요한 모든 primitive 를 포괄 |

### 실제로 새로운 코드

**4파일 수정 / 1파일 추가.**

1. **`crates/vrf-container/src/lib.rs`** (~60행 추가)
   `pub struct CheckpointMeta { id, group, metadata, time1, time2, size_in_bytes, archive_offset }`
   더하기 `pub fn parse_checkpoint_meta(payload: &[u8]) -> Result<CheckpointMeta, ContainerError>`
   와 `pub fn decompress_checkpoint(payload: &[u8], compressed, encrypted) -> Result<Vec<u8>, ...>`.
   마지막은 16바이트 접두 합성 트릭을 쓰지 *않아야* 한다 -- Oodle 본문을
   `decompress_replay_data` 에서 빼내 private `decompress_oodle_archive(bytes, expected_len)`
   로 만들고 양쪽이 호출. 체크포인트 고유 크기 불일치용 `ContainerError` 변형도 추가.

2. **`crates/vrf-schema/src/checkpoint.rs`** (신규, ~150행 + 테스트)
   - `pub fn read_checkpoint_guid_cache(reader, cache) -> Result<u32>` -- §2.2, 엔트리 수 반환.
   - `pub fn read_checkpoint_export_group_map(reader, cache) -> Result<u32>` -- §2.3.
   - `pub fn read_checkpoint_tables(data: &[u8], cache) -> Result<CheckpointTables>` returning
     `{ guid_count, group_count, frame_offset }` where `frame_offset` is the post-map cursor.
     Assert `frame_offset == u32::from_le(data[0..4]) + 8` and error if not -- 0 예외의 4,024
     표본이 뒷받침하는 무료 무결성 검사.
   New `SchemaError` variants: `UnexpectedPathKind { byte }`, `CheckpointOffsetMismatch { .. }`.
   Export from `crates/vrf-schema/src/lib.rs`.

3. **`crates/vrfkit/src/driver.rs`** (~30행)
   `ChunkType::Checkpoint` 팔 추가. 체크포인트 테이블이 자기 완결적이므로 올바른 형태는
   **체크포인트마다 별도 `NetGuidCache`** (그 체크포인트 자체 테이블에서 시드) 와 **별도
   `ReplicationReader`** 다 -- 체크포인트 익스포트를 메인 ReplayData 캐시에 넣지 *말* 것.
   이유: (a) 체크포인트 맵의 `path_name_index` 값이 ReplayData 와 같은 번호에서 뽑히므로
   합치는 것이 *아마도* 안전하지만 검증되지 않았고, (b) 액터 채널 상태는 커넥션별이라;
   라이브 리더로 채널 오픈을 재생하면 메인 스트림의 채널 테이블이 깨진다. 중요 경로가
   아니므로 CLI 플래그 (`--checkpoints`) 뒤에 둘 것.

4. **`crates/vrfkit/src/cli.rs`** (~5행) -- 플래그.

5. **싱크/익스포터** -- `crates/vrfkit/src/sink.rs` / `crates/vrf-export/*`: 무엇을 내보낼지
   결정. §6 의 발견 (체크포인트 내용이 ~100% 중복) 을 고려하면 최고 가치 배출은 필드가
   *아니라* **NetGUID->경로 테이블** (코퍼스 전체 17.2M 엔트리, 18개 표본 시각에서 모든 정적/동적
   오브젝트의 outer 체인 제공) 과 추가 46~51 그룹 경로일 것이다. 나머지는 중복 상태다.
   **이것은 포맷 문제가 아니라 제품 결정이며, 나는 내리지 않았다.**

### 기획용 비용 / 규모 수치

| 수량 | 코퍼스 합계 (215파일) |
|---|---|
| 체크포인트 청크 페이로드 바이트 | 1,062,150,914 (브리프 수치와 정확 일치) |
| 감압 후 아카이브 바이트 | 2,967,025,362 (2.97 GB -- 2.8x 팽창) |
| 체크포인트 | 4,024 |
| GuidCache 엔트리 | 17,186,645 |
| 익스포트 그룹 레코드 | 1,955,988 |
| DemoFrame | 4,024 (체크포인트당 정확히 1) |
| DemoFrame 패킷 | 904,891 |
| 감압 + 모든 테이블 파싱 + 모든 프레임 워크의 벽시계 | 싱글스레드 release 빌드 **~24초** |

---

## 8. 주장별 검증 방법

| 주장 | 검증 방법 |
|---|---|
| 청크 헤더 레이아웃, `Time1 == Time2`, id/group/metadata 명명 | 215파일 전체에서 체크포인트마다 단언 -- `cp2 verify`, 위반 0건 / 4,024건 |
| `compressed_size + 8 == SizeInBytes`, `decompressed_size == len` | 같은 실행, 위반 0건 |
| 프로로그 워드 4/8/12 가 0 | 같은 실행, 위반 0건 |
| guid 캐시 엔트리 레이아웃 | (a) 정확한 폐쇄: `NumGuidCacheEntries` 엔트리가 타당한 `NumNetFieldExportGroups` 위에 도달; (b) outer 체인이 합리적 패키지/월드/CDO 관계로 해석; (c) `PathIsString` in {0,1} 을 17,186,645 코퍼스 엔트리에서 파서가 강제, 위반 0건; (d) `Flags` in {0,3} 을 1,499,702 엔트리 (20파일, 미강제) 에서 관측 |
| 모든 아카이브 `FString` 이 UTF-16LE | `verify` 중 모든 길이 접두 부호를 집계: 25,038,008 음수, **양수 0**, 코퍼스 전체 |
| 그룹 레코드의 `bExported` in {0,1} | 파서가 세 번째 값에서 하드 에러; 1,955,988 그룹 레코드 위반 0건 |
| 익스포트 그룹 레코드 레이아웃 | `map_end == w0 + 8` 의 정확한 폐쇄가 4,024/4,024, `Handle == slot index` 가 코퍼스 전체 11,529,869/11,529,869 익스포트 슬롯 |
| `NumNetFieldExports` 가 IntPacked | 결정적: u32 면 첫 그룹이 42 슬롯을 주장하고 오프셋 139,401 의 다음 그룹 FString 로 넘침; IntPacked 면 21 을 주장하고 정확히 거기서 닫힘 |
| 체크포인트당 정확히 한 DemoFrame | probe 의 독립 프레임 워커 (`walk_frames`, `vrf-frame` 문법 미러), 4,024 프레임 / 4,024 체크포인트 |
| 프레임 시각 == 청크 `Time1` | 같은 워커, 1 ms 이상 벌어진 프레임 0건 |
| `iter_demo_frames` 와 빈 캐시로 프레임 파싱 | 참조 리플레이에서 체크포인트마다 실행: `Ok(64)`...`Ok(261)` |
| 오프바이원 진단 | `w0+0 ... w0+10` sweep, §4 에 정리 |
| Q4 부정 결과 | 4,024 체크포인트 전체의 1,955,988 파싱된 그룹 *경로* (raw 바이트 스캔이 아님) 위 `AbilitiesAndBuffs` 부분문자열 -> 0 |
| 체크포인트 스키마 ~= ReplayData 스키마 | `(group, handle) -> name` 집합 비교, 참조 리플레이 |
| 액터 집합 겹침 | `ReplicationReader` 액터-오픈 GUID 집합, cp0 와 cp5 vs. ReplayData 스트림 전체 |

probe 명령 (모두 `cp2` 안): `list`, `hex`, `gaps`, `gc`, `map`, `full`, `fullall`, `verify`,
`stats`, `kind0`, `offbyone`, `q4b`, `delta`, `q5`.

---

## 9. 기각된 가설과 정정

반복을 막기 위해 기록.

1. **"프레임은 `w0 + 7` (169,593) 에서 시작."** 틀림. `w0 + 8`. `+7` 은
   `(i32 in 0..8, f32 in 0.1..3600)` 의 무차별 스캔에서 왔으며, 200 KB 안에 오탐이 많다.
   오프셋을 익스포트 그룹 맵의 끝에서 유도할 것.
2. **"익스포트 캐시를 ReplayData 에서 체크포인트 시각까지 미리 채워야 한다."** 틀림.
   체크포인트 프레임은 net-field-export 레코드를 0개 담는다 (0/4,024).
3. **"체크포인트 익스포트 섹션은 변형 인코딩을 쓴다."** 틀림. 스톡 `iter_demo_frames`.
4. **"`u32 @16` 이 guid 엔트리가 아닌 무언가를 센다"** -- 첫 파싱이 159 엔트리 후 멈춰
   검토됨; 그 정지는 카운트가 틀려서가 아니라 `PathIsString == 0` 변형 처리 잘못이었다.
5. **guid 엔트리 머리의 쌍 `(a, b)` 는 `(NetGUID, OuterGUID)` 이지 `(Outer, GUID)` 도, 단일
   2바이트 packed 값도 아니다.** `03 0a` 를 하나의 `IntPacked` = 641 로 읽으면 해석 불가능한
   outer 체인; 올바른 분할은 이전 엔트리의 `Flags` 바이트가 먼저 오게 한다.
6. **`NumNetFieldExports` 는 `u32` 가 아니다.** u32 로 읽으면 작은 카운트에서 참값의 정확히
   2배 (IntPacked 가 1만큼 왼쪽 시프트하므로) 가 되어 조용히 넘친다.
7. **guid 캐시 엔트리는 정적 GUID 전용이 아니다.** 25% 가 짝수 (동적) GUID.
8. **C# 참조 파서는 체크포인트를 구현하지 않는다.** `ReplayChunkDispatcher.cs`
   (`src/Replay.Unreal/Chunks/ReplayChunkDispatcher.cs`, `case ReplayChunkType.Checkpoint:`
   팔)이 `"Skipping checkpoint chunk {ChunkIndex}."` 를 로그하고 더는 아무것도 안 한다.
   인용할 일차-소스 구현이 없다; 위의 모든 것은 바이트에서 유도했다.
   (`Replay.Encoding` 의 `ArchiveCheckpoint.cs` 는 관계없는 아카이브 저장/복원 헬퍼.)
9. **Unreal Engine 소스는 어떤 것도 참조하거나 의존하지 않았다.** 위의 필드 *이름*
   (`bNoLoad`, `NetworkChecksum`, ...) 은 편의상 붙인 라벨이며; 바이트 레이아웃과 값 분포만이
   증거다.

---

## 10. 미해결 / 미지수

- `PathIsString == 0` guid 엔트리의 `IntPacked` `NameIndex` 의 의미. 이름-테이블 인덱스임이
  (재사용 패턴으로) 검증됨; 테이블이 파일에 없어 리플레이만으로 텍스트 복원 불가. 같은
  제약이 ReplayData net-field 익스포트의 하드코드 FName 에 이미 존재하므로 회귀가 아니다.
- 바이트 0..16 이 두 `int64` 인지 네 `u32` 인지. 고위 워드가 0이라 결정 불가.
- 체크포인트 `path_name_index` 값이 ReplayData 와 같은 번호 공간에서 뽑히는지. 미테스트;
  구현 계획은 이에 의존하지 않도록 회피.
- 체크포인트의 개별 프로퍼티 값이 같은 타임스탬프의 ReplayData 값과 다른지 (즉 체크포인트가
  증분 스트림을 *정정* 하는 일이 있는지). 미측정.
- 체크포인트 프레임당 0-10 ClassNetCache 블록의 RPC 내용. 미디코드 (probe 싱크가
  `function_count = 0` 반환).
