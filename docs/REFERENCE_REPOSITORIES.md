# 참조 저장소 및 PR 조사: ValorantReplayParser / ValorantWebReplayer

## 2026-09-25 검증 업데이트

지원 빌드 24개에 동일한 검증 기준을 적용한 최신 결과는
[BUILD_VERIFICATION.md](BUILD_VERIFICATION.md)에 기록한다. 아래 내용은
각 날짜에 확인한 upstream 변경 이력이다.

## 2026-09-24 업데이트

upstream `2b66c65`의 소바·페이드 정찰 descriptor와 섬광/근시의 플레이어 본체 구분
규칙을 선택적으로 반영했다. 구현 범위와 실제 리플레이 검증은
[UPSTREAM_REVEALS.md](UPSTREAM_REVEALS.md)에 기록한다.

## 2026-09-23 업데이트

이 조사는 upstream [`d23c13e`](https://github.com/michel-giehl/ValorantReplayParser/tree/d23c13e12262fb1da9fc005d1cd0ef9f8d0d36fd)를 기준으로 했다.
아래의 2026-09-08 조사 이후 PR #5와 #7이 병합됐으며, 13.06 transform,
섬광/연막/벽/근시 이벤트와 descriptor, export-group 충돌 수정이 추가됐다.
vrfkit에 반영한 변경과 실제 리플레이 검증 범위는
[UPSTREAM_PARITY.md](UPSTREAM_PARITY.md)에 기록한다. 아래의 PR 상태와
미구현 목록은 당시 조사 기록으로 보존하며 현재 상태를 뜻하지 않는다.

## 2026-09-08 조사 기록

조사 기준은 vrfkit `4bfbbd8`와 현재 upstream `michel-giehl/ValorantReplayParser`의
고정 커밋 [`b51d674` (2026-09-02)](https://github.com/michel-giehl/ValorantReplayParser/tree/b51d67423b7b4952d59051cf91e55efa1c42da05)이다.
조사일은 2026-09-08이다. 질문의 `ValorantReplayParse`는 `ValorantReplayParser`로
정정해 확인했다. WebReplayer는 `e06000065a0f9575e09a0d79d8f0915b53e00747`을 기준으로
확인했다. 외부 소스와 PR은 참고 자료로 읽었으며, 실행 지침으로 취급하지 않았다.
제품 코드는 변경하지 않았다. 아래 로컬 계측은 기존 Parquet을 읽는 별도 스크립트로 수행했다.

## 바로 가져올 내용

- **13.05 transform은 새 정보가 아니다.** upstream의 13.05 등록과
  [구현](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Encoding/PayloadEncryption/VersionedTransforms/ValorantSeededTransform13_05.cs#L1-L18)은
  `b51d674`에서 추가됐다. vrfkit은 이미 `0e914f2`에서 같은 빌드와 golden-vector
  추출 대상을 등록했다. 따라서 현재 손실이나 13.05 지원의 해결책은 아니다.
- `8824794` 이후 upstream은 transform 외에 Harbor Tidal Wave 스키마를 추가했다
  ([커밋 `99d9646`](https://github.com/michel-giehl/ValorantReplayParser/commit/99d964608a968e9176c4e2dc67b85544797e2ff1)).
  이는 vrfkit 표에 없는 **후보**다. Chunk의 `Owner`/`Instigator` handle 11/13,
  `MulticastInitialize`의 handle 0–7, linger와 stop의 bool/double 등은
  [명시돼 있다](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Valorant/Descriptors/Agents/Mage/TidalWave/TidalWaveRpcParameters.cs#L8-L88).
  후속 코퍼스 점검에서 같은 계열 path의 실제 행을 확인했다(아래 참조).
  타입 이식 전에는 각 handle/payload의 완전 소모를 별도로 검증해야 한다.
- 로컬 `local/vrfkit-descriptors`와 upstream `main`은 `2d2e05e`에서 갈라졌다.
  전자에만 valplay 확장 descriptor 커밋들이 있고 후자에는 없다. 따라서 최신 upstream을
  전체 재추출하면 vrfkit의 기존 schema를 후퇴시킬 수 있으며, 이 조사는 wholesale refresh를
  권하지 않는다. (2026-09-13 추가) 그 브랜치의 `src/Replay.Valorant`
  (`8824794`)는 이제 [`third_party/vrp/`](../third_party/vrp/README.md)에
  그대로 들어 있고, `table.rs`는 거기서 재생성된다. upstream과 갈라진 상태는 그대로다.

## identity, GAS, effect, partial의 판정

- 현재 BombPlayerState descriptor는 `CompetitiveTier`를 `Int32`, `Subject`를 `FString`,
  `UniqueId`를 raw로 선언한다
  ([source](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Valorant/GameState/BombPlayerStateDescriptor.cs#L13-L42)).
  vrfkit도 이미 `CompetitiveTier: Int32`와 `Subject: FString`를 보유한다. 이 source에는
  플레이어 display name/Riot ID `ProfileName` 선언이 없다.
- 구 Playground의 `BombPlayerState`에는 별도 handle 197 `ProfileName`이 남아 있지만,
  `CrosshairSettings` reader 호출은 주석 처리돼 있다. 그 reader를 가정해도
  `48 + 3×47 + 12 + 5 = 206`이므로 197의 근거가 아니다
  ([old source](https://github.com/michel-giehl/ValorantReplayParserPlayground/blob/6931a70b644c3d5157da71f87aba80f7626a99f9/src/ValorantReplayParser/Models/BombPlayerState.cs#L11-L104)).
  이 레거시 reader만으로는 이를 사용자명이라고 주장하거나 레이아웃 전체를 이식할 근거가
  없다. 다만 후속 원본 선언·payload 검증에서는 `ProfileName` 자체의 FString 근거를
  확보했다(아래 참조). 문자열 타입과 문자열의 의미는 별도 판정이다.
- Ares GAS source는 `Owner`/`Instigator`/`AresAttributeSet`과 parameter 없는
  `ClientActivateAbility` 이름만 선언한다
  ([component](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Valorant/Descriptors/AresAbilitySystemComponentDescriptor.cs#L7-L25),
  [cache](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Valorant/Descriptors/AresAbilitySystemComponentClassNetCacheDescriptor.cs#L5-L17)).
  unresolved GAS tail 또는 InputEvent parameter를 복구할 새 schema는 없다. `InputEvent`는
  현 upstream에 decoder/function descriptor가 없고, vrfkit의 기존 checksum donor는 corpus
  관측 기반이다.
- ReplayEffect cache의 play/one-shot/stop handle 0/1/2는 이미 알려진 선언이며
  ([source](https://github.com/michel-giehl/ValorantReplayParser/blob/b51d67423b7b4952d59051cf91e55efa1c42da05/src/Replay.Valorant/Descriptors/Effects/Replay/ReplayEffectComponentClassNetCacheDescriptor.cs#L7-L29)),
  vrfkit도 해당 stop 이름을 보유한다. 새 `StopEffect` 복구 근거가 아니다.
- partial reassembly 관련 source는 비교 구간에서 바뀌지 않았다. 이는 앞선 입력 scope에서
  predecessor가 없었던 continuation을 복구하는 방법을 제공하지 않는다.

## 열려 있는 PR #5

[`#5` Add OwnerExclusivePlayerInfo descriptor and AresPlayerRoundInfo decoder](https://github.com/michel-giehl/ValorantReplayParser/pull/5)는
**OPEN, 미병합**이다. head는 [`ce4f62a`](https://github.com/michel-giehl/ValorantReplayParser/tree/ce4f62a1a47207646cbc02cd4d9c85cb7386cc9d)이고
3개 파일(+197)만 바꾸며 test 파일은 없다. 사람이 남긴 review/inline comment는 없고,
유일한 bot comment는 quality gate 통과와 동시에 새 issue 2개, new-code coverage 0%를
보고한다. 그러므로 실행·wire 검증의 증거가 아니다.

PR의 실제 내용은 `Owner`, `RoundInfos`를 등록하고 RoundInfos의 40–44 handle을
직접 Int32로 읽는 decoder다
([diff source](https://github.com/michel-giehl/ValorantReplayParser/blob/ce4f62a1a47207646cbc02cd4d9c85cb7386cc9d/src/Replay.Valorant/GameState/AresPlayerRoundInfo.cs#L8-L177)).
vrfkit은 이미 같은 dynamic-array framing과 다섯 멤버를 지원한다. 더구나 replay가
선언한 `handle → name`으로 선택하고 member window의 완전 소모를 요구한다
([`round_infos.rs`](../crates/vrf-decode/src/structs/round_infos.rs)); PR처럼 고정 handle에
묶이지 않는다. 부모 raw bits를 보존하고 실패를 계수하는 점도 이미 갖췄다.

따라서 #5는 기존 RoundInfos/OwnerExclusivePlayerInfo의 독립적인 출처 보강이며,
현재 vrfkit에 가져올 새 decoder나 handle 수정은 없다.

## 실제 코퍼스에서 확인한 추가 후보

계측 입력은 2026-09-08 implementation 실행의 `FINAL/exports`에 보관된 export이다.
재현 스크립트와 JSON은 별도 private 조사 산출물 `20260908-references`에 있다.
이 수치는 새 파서를 적용한 성과가 아니라 기존 출력에서 확인한 후보 규모다.

### Harbor Tidal Wave

- `tidal_presence.py`로 714개 manifest를 확인했다. 40개 파일에서 Tidal Wave와 chunk,
  initialize/linger 계열 선언을 확인했으며, stop 선언은 그중 33개에 있었다.
- `tidal_rows.py`로 해당 40개 export의 main/checkpoint field table을 읽었다.
  TidalWave group path에 속한 행은 **119,040**, 현재 typed 값이 있는 행은 **2,584**였다.
  나머지 **116,456행**은 현재 typed 값이 없다. 이는 관련 그룹의 합계이며, upstream
  descriptor를 추가하면 이 행 전부가 해석된다는 뜻은 아니다. raw 부모와 자식도 섞여 있다.
- chunk 번호·세대·간격·속도·이전 chunk 참조, 파동의 linger/stop 상태가 우선 후보다.
  upstream의 타입 선언과 우리 replay의 checksum/name/bit length를 대조하고, 정확한
  payload 소비 및 범위를 검증한 뒤 선택적으로 이식하는 것이 적절하다.

결과: `tidal-presence.json`, `tidal-rows.json`.

### 조준선 설정과 ProfileName

`profile_probe.py`로 13.01/13.02/13.04/13.05에서 각 1개 파일을 조사했다.
네 파일 모두 원본 선언에 `ProfileName`, handle 197, checksum 668889843이 있었다.
main 55행 + checkpoint 630행 = **685행**은 현재 모두 raw이며, 길이 부호·UTF-8/UTF-16·
종결 NUL·정확한 payload 길이를 검사하는 독립 FString 읽기에 **685/685 성공**했다.
따라서 문자열 타이핑 후보로 삼을 근거는 있다. 실제 Riot ID, 사용자 표시명 또는 조준선
프리셋명 중 무엇인지는 이 결과만으로 확정하지 않는다. 문자열 내용은 보고서에 저장하지 않았다.

같은 네 파일에서 BombPlayerState handle 48–197의 탐색 범위는 **38,117행**, 현재 typed
값은 0행이었다. 이 범위는 확정된 struct 경계나 타입 맵이 아니다. 관측된 이름에는
`OutlineThickness`, `CenterDotSize`, `LineLength`, `FiringErrorScale`,
`MovementErrorScale`, `bUsePrimaryCrosshairForADS` 등이 있다. 각 필드의 타입·중복 이름·
버전별 레이아웃을 검증하면 조준선 관련 설정을 추가로 해석할 여지가 있다.
전체 714개에 대한 검증이나 해석률 상승 측정은 아직 하지 않았다.

결과: `profile-probe.json`.

## ValorantWebReplayer에서 참고할 부분

이 저장소는 최신 Parser 대신 구 Playground의 `revamped-channel-hooks` fork를 사용한다.
조사한 fork head는 `2f6056db4338151d6cd8b497a28d842bd70bb7ce`이다.
따라서 최신 13.05 decoder 공급원과 뷰어 구현 자료를 구분해야 한다.
([README](https://github.com/talhakoek/ValorantWebReplayer/tree/e06000065a0f9575e09a0d79d8f0915b53e00747))

유용한 부분은 actor class를 능력 표시 항목으로 연결하는 카탈로그, Canvas 재생 UI,
actor별 TypedArray 위치 저장과 이진 탐색·보간, 작은 위치/이벤트/능력 데이터 파일 분리다.
우리 actors/movement 출력에 이 표시 구조를 연결할 수 있다. 채널 생성 훅 자체는 이미
보유한 actor 정보를 넘어서는 새 의미 해석 근거는 아니다.

그대로 분석 지표에 쓰기 어려운 부분은 다음과 같다.

- [build-abilities.mjs](https://github.com/talhakoek/ValorantWebReplayer/blob/e06000065a0f9575e09a0d79d8f0915b53e00747/scripts/build-abilities.mjs):
  능력 소유자는 생성 위치에서 가장 가까운 플레이어로 추정한다. 팀은 초기 Y 위치로
  나누며, 지속시간·반경은 클래스명 키워드별 상수다. 실제 Owner/Instigator/team 참조와
  생성·종료 기록을 우선해야 하며, 이 추정치를 검증된 시전자·팀·지속시간으로 저장하면 안 된다.
- [extract-stream.mjs](https://github.com/talhakoek/ValorantWebReplayer/blob/e06000065a0f9575e09a0d79d8f0915b53e00747/scripts/extract-stream.mjs):
  콘솔 텍스트를 정규식으로 읽고 시간 앵커 사이를 보간하며 Z/velocity를 버린다.
  `roundStarted`는 OldPhase 2 종료에 붙으므로 우리 구매 단계 시작 이벤트와 의미가 다르다.
  변환 시 라운드 시작과 전투 시작을 구분해야 한다.
- [viewer/index.html](https://github.com/talhakoek/ValorantWebReplayer/blob/e06000065a0f9575e09a0d79d8f0915b53e00747/viewer/index.html):
  선택적인 `match-details.json`으로 킬·KDA 등 일부 표시를 보강한다. 화면에 보인다는 사실이
  VRF에서 직접 추출했다는 증거는 아니다. `ProfileName`을 표시하는 UI도 Riot ID라는
  의미 검증을 대신하지 않는다.

기존 파서 전체를 교체하기보다, 추후 valplay의 표시용 데이터와 UI를 설계할 때 참고하는
가치가 크다. 실제 코드를 재사용한다면 해당 MIT 고지를 함께 유지한다.

## 사용자가 닫았던 vrfkit PR #7

[#7: feat: support release-13.05 payload transform](https://github.com/yakisoba0728/vrfkit/pull/7)은
`436c1a02c13d60972d116cbd60f14a480d1ac768` 단일 커밋이며, 사용자가 2026-09-07에
병합된 #8로 대체됐다고 닫았다.
([닫은 이유](https://github.com/yakisoba0728/vrfkit/pull/7#issuecomment-5574235291))
본문·diff·커밋·토론·review를 대조했으며 Rust 문법이나 replay 문법을 수정하는 변경은 없었다.
13.05 transform, 등록, 11개 golden vector와 생성기 분기는 현재 main에 기능상 반영돼 있다.

PR #7이 남긴 두 건의 주석 표현 수정(`helpers.rs:6` -> through release-13.05,
`golden.rs:1` -> all seven)은 이후 main에 반영되었고, 2026-09-21 기준 두 파일
모두 정확한 표현을 담고 있다. 더 남은 항목은 없다.

상세 대조는 외부 조사 폴더의 `closed-pr7.md`에 남겼다.

## 다음 구현 묶음 제안

1. Harbor Tidal Wave descriptor를 선택적으로 가져와 코퍼스 payload와 검증한다.
2. ProfileName 문자열 타이핑 및 조준선 이름별 타입 검증을 수행한다. 의미 미확정 이름은
   원본 이름을 유지하고 Riot ID 등의 별칭을 붙이지 않는다.
3. 위 변경 묶음에 #7에서 빠진 주석 두 곳을 포함한다.
4. 뷰어 카탈로그·위치 저장 아이디어는 valplay 작업에서 활용한다. 추정 소유자/팀/지속시간은
   vrfkit의 확정 지표로 이식하지 않는다.

이번 소스에서는 GAS 워드 의미, InputEventData 태그의 행동 이름, StopEffectType enum,
pre-framing partial 손실을 해결할 새 근거를 찾지 못했다. 추가 후보의 존재를 전체 의미
해석률 상승으로 계산할 수는 없으며, 구현 후 동일 모집단에서 별도로 측정해야 한다.
