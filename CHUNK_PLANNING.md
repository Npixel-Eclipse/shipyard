# Tracking chunk 계획

최초 작업의 기준은 Npixel-Eclipse/shipyard의 `origin/0.11.5`, 커밋 `d3dbd51435c225629564f3eda723291747aebda1`이다. 리뷰 브랜치 `0.11.5-chunk-planning`은 `0.11.6`과 별도로 개발했으며, `0.11.7` 통합 내용은 다음 절에 기록한다. 아래 Battle 비교를 위해 임시 적용한 의존성·설정·배포 바이너리는 측정 후 모두 복구했다. 별도 patch5 저장소도 그대로 유지했다.

아래 기존 실측과 최초 검증 기록은 `b7290af` 기준이다. 후속 작업은 다음 절에 구분하며, 기존 실측 수치를 후속 코드의 성능 결과로 해석하지 않는다.

## 0.11.7 통합

`0.11.7`의 `725d8d6`에 `0.11.5-chunk-planning`의 `45646a2`를 통합한다. 버전은 `0.11.7`을 유지하며, 기존 다섯 수정인 복제 시 tracking 청크 요약 복원, 역순 ID 대응, 정렬 시 tracking 보존, 삭제·제거 기록 비우기, OR `with_id().fold()` 전환을 함께 보존한다.

순회 계획은 아래의 `/2` 예산과 `Empty`·`Dense`·`Sparse` 경로를 사용한다. `0.11.6`의 `/8` 빈 입력 검사 정책과 중복 적용하지 않는다. 일반 조인의 captain 선택과 결과 순서는 달라질 수 있으며, 반환 집합·ID/값 대응·명시한 entity-id slice 순서를 검증한다. 첫 번째 결과에 따라 처리 대상을 결정하는 호출 코드는 순서 변경의 영향을 받을 수 있다.

사용자 정의 iterator의 소스 호환성을 위해 `ShiperatorCaptain::is_definitely_empty`는 유지하되, iterator 생성 시 자동 호출하지 않는다. 사용자 정의 빈 결과 최적화는 미리 만든 계획을 `has_no_candidates`로 노출해야 한다. 계획 생성 예산을 전달하는 wrapper는 `planning_len`과 `into_shiperator_with_budget`을 구현한다. 기존 구현은 기본 메서드를 통해 일반 순회를 계속한다.

통합 검토에서 `((a.inserted() | !b.modified_mut()) | &c)`가 B/C 공유 엔티티를 두 번 반환하는 문제를 추가로 재현했다. B를 반환한 뒤 수정하면 `!modified`가 거짓으로 바뀌어 C 쪽의 중복 검사를 통과했다. 수정 추적으로 왼쪽 조건이 바뀔 수 있는 OR은 왼쪽에서 조건을 통과한 ID를 기억하고, 오른쪽에서 현재 조건과 이 기록을 함께 확인한다. 추가 메모리는 기억한 ID 수에 비례한다. 안정된 조건의 OR에는 기록용 할당을 추가하지 않으며, 기존 단일 producer 제한은 유지한다. 이 기록은 내장 수정 추적 조건의 중복 반환을 막으며, 사용자 정의 필터 전체를 시작 시점 상태로 고정하지는 않는다.

이 통합에서 Battle 실행이나 성능 재측정은 하지 않는다. 아래 과거 측정치는 통합본의 성능 결과가 아니다.

통합 검증(2026-09-08):

- `cargo test -p shipyard --all-features --locked`: 일반 테스트 340개와 문서 예제 83개, 총 423개 통과.
- `cargo test -p shipyard --no-default-features --features std,proc --lib --test lib --test tracking_chunks --test chunk_planning --test clone --test sort --test delete --test or_with_id_fold --test or_mutable_membership --locked`: 188개 통과.
- `cargo check -p shipyard --no-default-features --locked`: 통과.
- `cargo clippy -p shipyard --all-features --all-targets --locked -- -A clippy::mut_from_ref`: 에러 없음. 기존 `custom_storage.rs`의 `mut_from_ref` 오류 때문에 이 lint를 허용한 구성이며, 기존 경고는 유지한다.
- 새 이력 구현과 회귀 파일의 `rustfmt --check`, 통합 diff의 `git diff --check`: 통과. 기존 파일의 별도 포맷 차이는 보존한다.

## 근사화 없는 후속 최적화

- 64슬롯 청크, captain 비용식, Dense fallback 기준은 유지한다. Dense가 확정되는 순간 나머지 metadata 검사를 중단한다. 예를 들어 782청크에서 앞쪽 391청크가 후보이면 그 시점에 기존 Dense 경로로 돌아간다. 미검사 구간을 빈 구간으로 간주하지 않는다.
- `Empty`와 `Dense`는 값으로 보관하고 `Sparse`만 `Arc`로 공유한다. 빈 계획의 heap 할당과 producer clone의 빈 계획 참조 카운트 갱신을 제거한다. 후보 연속 구간 수를 metadata 검사 중 계산해 구간 목록을 한 번에 할당하며, 분할 지점 계산의 중복 rank 조회를 제거한다.
- OR의 빈 입력 증명은 양쪽 모두 비었을 때만 전파한다. source별 계획을 정방향·역방향 청크 건너뛰기와 후보량 계산에 연결한다. OR membership 조회는 왼쪽 조건이 성립하면 오른쪽 조회를 생략한다.
- 중첩 OR의 모든 source를 보존한다. 다른 source에서 온 entity의 membership 조회는 현재 captain 상태와 무관하게 전체 조건을 검사한다. `Modified`, `InsertedOrModified`, mutable tracking 입력과 OR 자체의 `|` 조합을 지원하며, View 재대여 수명과 storage 수명을 분리한다.
- 병렬 OR은 먼저 source 경계에서 나누고 각 source 내부를 후보량으로 나눈다. 빈 source는 작업 분할 전에 생략한다. 기존 `RawEntityIdAccess::split_at`의 후속 구간 내부 절단에 의존하지 않는다. 후보량은 중복 제거 전 상한이며, callback 비용의 추정이나 샘플링은 추가하지 않았다.
- 중복 검사에서 다른 producer가 수정할 수 있는 per-component modification timestamp를 읽는 조합은 단일 producer로 실행한다. 예를 들어 `a.modified_mut() | b.modified_mut()`가 해당한다. 읽기 전용 tracking OR과 일반 mutable OR은 병렬 분할을 지원한다. 이 제한을 풀려면 별도의 안정된 membership snapshot 설계가 필요하다.
- 역방향 OR은 마지막 source부터 소비하며, 앞뒤 혼합 순회에서도 source별 남은 구간과 `with_id`의 entity/value 대응을 유지한다. 기존 버그가 있던 역방향·중첩 OR의 동작은 수정된다. captain 변경에 따른 정방향 순서 차이라는 기존 제약은 그대로다.

후속 회귀 테스트는 Dense 검사 중단 경계, bitmap word 경계, 빈 OR·희소 source·중복·Optional/Not·좌우 중첩 OR·정방향/역방향/부분 소비, 1/4/16 worker의 결과 집합과 mutable tracking 분할 제한을 검사한다. 실제 Battle 실행이나 성능 재측정은 이 후속 작업에 포함하지 않는다.

OR·메모리 최적화 단계 검증(2026-09-08, 아래 예산 도입 전):

- `cargo test -p shipyard --all-features`: 385개 통과.
- `cargo test -p shipyard --no-default-features --features std,proc --lib --test tracking_chunks --test chunk_planning`: 87개 통과.
- `cargo check -p shipyard --no-default-features`: 통과.
- `cargo clippy -p shipyard --lib --tests`: 에러 없음. 기존 파일의 acronym, identity operation, qualification 등 경고는 유지한다.
- 변경 Rust 파일의 `rustfmt --check`와 `git diff --check`: 통과.

## 계획 생성 예산

0.11.6의 검사 예산 개념을 계획 생성 앞단에 적용한다. metadata를 읽지 않는 `planning_len`으로 임시 driver 길이를 구하고, 각 positive tracking 입력에 `max(driver_len / 2, 1)`청크를 허용한다. driver 길이가 0이면 예산도 0이다. `/2`는 이 브랜치의 sparse captain 선택·병렬 분할 이득을 유지하기 위한 비용 제한 정책이며, 측정된 시간 비율이나 최적값을 뜻하지 않는다. 0.11.6의 빈 입력 검사 전용 `/8`을 그대로 사용하지 않는다.

- AND는 captain이 될 수 있는 입력들의 길이 중 최소값을 사용한다. 필수 entity-id slice가 있으면 다른 storage가 더 작아도 그 slice 길이를 우선한다.
- OR은 양쪽 source의 길이를 합산한다. 중첩 AND/OR은 상위 예산을 전달받아 필요하면 줄이며, 다시 늘리지 않는다. Optional과 `!&view`는 driver 길이 후보에서 제외한다. `!view.modified()`처럼 captain이 될 수 있는 tracking 부정 입력은 길이 후보에 포함한다.
- `ceil(storage_len / 64)`가 예산을 넘으면 timestamp를 읽거나 bitmap·구간 목록을 할당하기 전에 `Dense`로 돌아간다. 이는 미검사 상태를 보수적으로 순회한다는 뜻이며, `Empty` 판정이나 결과 생략에 사용하지 않는다. 실제 순회에서는 기존 component별 tracking·join 검사를 수행한다.
- 예산은 tracking 입력별 제한이다. 여러 입력의 검사량을 합산한 전역 예산은 아니다. 예산 이하에서는 기존 Dense 조기 종료와 정확한 후보 계획을 그대로 사용한다.
- 5만 컴포넌트는 782청크다. driver 64개는 32청크 예산이므로 사전 검사를 생략하고, driver 3천 개는 1,500청크 예산이므로 전체 계획을 허용한다. 경계는 driver 1,563개/1,564개다.
- 기존 custom `IntoShiperator`는 기본 메서드로 기존 생성 방식을 유지한다. custom wrapper가 내부 tracking 입력까지 예산을 전달하려면 `planning_len`과 `into_shiperator_with_budget`을 구현한다.

샘플링이나 변경 이력 추정은 추가하지 않는다. 예산 때문에 상세 계획이 생략되면 captain 선택과 결과 순서는 달라질 수 있으나, 반환 집합과 강제 entity-id slice의 순서는 보존한다. 이 예산은 사전 계획 생성 비용을 제한하며 실제 순회 전체의 비용을 제한하지는 않는다.

예산 도입 후 검증(2026-09-08): 전체 feature 테스트 389개, 직렬 구성 테스트 91개, no-std 컴파일이 통과했다. 예산 경계·metadata 미접근·중첩 예산 전달·강제 slice·OR 전체 길이·Optional/Not·mutable fallback을 추가 검증했다. Clippy는 에러 없이 기존 경고만 유지한다. 변경 파일의 rustfmt 검사와 diff 공백 검사도 통과했다. 성능 재측정은 수행하지 않았다.

## 변경

기존 64슬롯 insertion/modification chunk timestamp를 현재 View의 tracking 구간으로 판정해 iterator 수명 동안의 계획을 만든다. SparseSet 갱신 시 추가하는 카운터나 전역 통계는 없다.

- `TrackingPlan`: Empty, Dense(일반 순회로 fallback), Sparse 구분. Sparse는 후보 chunk bitmap과 연속 후보 구간/누적 슬롯 수를 가진다.
- `PlannedTracking`: positive Inserted/Modified/InsertedOrModified 입력에만 계획을 붙인다. 일반 iterator의 자료구조와 순회 루프에는 계획 필드를 추가하지 않는다.
- captain 비용은 전체 길이의 고정 2배 대신 후보 슬롯 상한 + 후보 chunk 수다. Dense fallback은 전체 길이 + 전체 chunk 수로 보수적으로 평가한다. 실제 시간이나 실제 join 결과 수를 뜻하지 않는다.
- Mixed AND의 필수 입력이 확실히 비면 전체 iterator 길이를 0으로 만든다. patch5의 빈 join 생략을 같은 계획으로 수행한다. Optional/Not/Or에 이 증명을 잘못 전파하지 않는다.
- 일반 순회는 후보 bitmap으로 깨끗한 구간을 건너뛴다. 후보 구간 안에서도 기존 component별 timestamp와 join 조건을 검사한다.
- par_iter는 후보 슬롯 상한으로 분할 하한을 계산하고, 누적 후보 수가 절반이 되는 지점에서 분할한다. 기존 `threads * 4`와 최소 16슬롯 정책은 유지한다. 작업 수를 스레드 수로 고정하지 않는다.
- 선택되지 않은 tracking 입력은 상세 계획을 해제한다. 선택된 sparse 계획은 producer들이 Arc로 공유하므로 분할 때 구간 목록이나 component 데이터를 복사하지 않는다.

큰 입력에서 후보 chunk가 절반 이상이면 일반 경로로 fallback한다(현재 최소 32 chunk부터 적용). 이는 후보가 넓게 흩어졌을 때 구간 계획 비용이 이득을 상쇄하는 것을 제한하기 위한 초기 정책이다. Dense는 모든 component가 변경됐다는 뜻이 아니다. 정확한 조건 검사는 여전히 실행한다. 작은 한 chunk 입력은 구간/bitmap을 할당하지 않는다.

## 정합성과 제한

- 메타데이터가 없거나 부족한 chunk는 후보로 처리한다. 과거 실행 통계로 이번 입력을 생략하지 않는다.
- bitmap은 timestamp를 한 번 읽어 캡처한다. 후보 구간은 이 bitmap에서 만든다. captain 선택, 빈 입력 확인, 순회, 분할이 같은 정보를 쓴다.
- tracking 관측 기준이 바뀌면 새 iterator에서 계획을 다시 만든다. 다른 시스템이나 다음 tick에 재사용하는 전역 캐시가 아니다.
- 후보 chunk의 슬롯 수는 상한이다. 실제 tracking 통과 수, 다른 component와의 교집합 크기, 사용자 callback의 비용은 아직 학습하지 않는다.
- captain이 바뀌면 entity 순회 순서가 달라질 수 있다. 반환 집합과 entity/value 대응은 보존하지만 이전 captain의 순서를 보장하지 않는다. 4,096개 입력에서 변경값 3·17을 join하는 재현에서 0.11.6은 `[17, 3]`, 이 구현은 `[3, 17]`을 반환했다. 첫 대상을 소비하는 호출부의 동작 검토가 필요하다. entity-id slice가 captain을 강제하는 경로는 유지한다.
- 0.11.6의 검사 예산 개념을 위 "계획 생성 예산" 절처럼 적용했다. 상세 계획 생성 전에 길이 정보만으로 예산을 결정한다.
- reverse iteration에서도 tracking captain을 검사하도록 수정했다. WithId::next_back의 entity index와 WithId::fold의 OR source 전환도 회귀 테스트 범위에 맞춰 바로잡았다.
- 최초 `b7290af`에는 OR 병렬 지원을 포함하지 않았다. 후속 지원 범위와 분할 방식은 위 절을 참고한다. workload batch fast path나 Battle 시스템 코드는 변경하지 않았다.

## 검증

회귀 테스트는 빈 필수 입력/강제 captain, tracking captain 선택, sparse tail 양분, chunk 및 bitmap word 경계, tracking 시각 override, inserted-or-modified, mutable iteration, 역방향/부분 순회, Optional/Not/Or, 1/4/16 worker의 결과 집합을 확인한다. 기존 tracking의 swap-remove/bulk 경로도 함께 실행한다.

원본 커밋에 새 테스트를 적용해 captain 선택과 sparse tail 분할 테스트가 실패하고, 개선 코드에서 통과함을 확인했다. 일반 결과 집합 비교는 원본에서도 통과했다.

전체 feature 테스트 378개, 직렬 구성 테스트 82개가 통과했으며 no-std 컴파일을 확인했다. 0.11.6에서 추가한 12개 테스트를 이 구현에 연결했을 때 11개가 통과하고 검사 예산 경계 테스트 1개가 실패했다. 후자는 결과 누락이 아니라, 0.11.6이 큰 tracking 입력의 검사를 생략하는 경우에도 이 구현이 요약을 생성해 빈 iterator로 만드는 정책 차이다.

```powershell
cargo test -p shipyard --all-features
cargo test -p shipyard --no-default-features --features std,proc --lib --test tracking_chunks --test chunk_planning
cargo check -p shipyard --no-default-features
cargo clippy -p shipyard --lib --tests
cargo clippy -p shipyard --all-features --all-targets -- -A clippy::mut_from_ref
```

all-features clippy의 원래 명령은 변경하지 않은 `all_storages/custom_storage.rs`의 기존 `mut_from_ref` 오류 6개에서 실패한다. 마지막 명령은 그 기존 lint만 허용한 검사다. 일반 feature clippy는 별도 허용 없이 검사한다. 최종 로그는 `target/chunk-planning-validation`에 보존한다.

## Battle 비교 실측 — 2026-09-08

이 구현을 먼저, 새 Shipyard 0.11.6(`0f874181a44373e88ce10d2834392687c68fce68`)을 다음에 각 한 번 실행했다. Battle·Proto·Resource는 CL 68179를 공통 사용하고, 해당 CL의 profiler 필터 변경도 양쪽에 동일하게 적용했다. Shipyard 의존성만 달랐다. 측정 시점에 고정한 소스 스냅샷과 리뷰 코드가 동일함을 확인했으며 이후 변경은 이 문서 갱신이다.

조건은 newbie 3,000명, 실행당 1,200초, 실행별 새 계정, local Kafka, no WPR였다. Tracy·상세 runtime profiling·BATTLE_TRACE·timeline은 비활성화했다. Ryzen 7900X3D 12코어/24스레드, SMT 켬, Rayon 16, Tokio 4, pin 끔, Normal priority, 목표 60 FPS를 유지했다. 빌드 옵션은 `--release --no-default-features --features push-metrics`였다.

| 구간 | 구현 | FPS | CPU 코어 상당량 | CPU ms/tick | 최소 PC | 평균 entity | 평균 active AI |
|---|---|---:|---:|---:|---:|---:|---:|
| 5~15분 | chunk-planning | 42.38 | 7.03 | 166.0 | 3000 | 109619 | 2650 |
| 5~15분 | 0.11.6 | 25.94 | 6.01 | 231.9 | 3000 | 114225 | 2696 |
| 15~19분40초 | chunk-planning | 27.33 | 7.07 | 258.7 | 3000 | 93412 | 1787 |
| 15~19분40초 | 0.11.6 | 16.55 | 6.05 | 365.6 | 3000 | 92914 | 1968 |
| 접속 완료+60~660초 | chunk-planning | 42.73 | 6.99 | 163.5 | 3000 | 112381 | 2645 |
| 접속 완료+60~660초 | 0.11.6 | 25.81 | 6.05 | 234.6 | 3000 | 111074 | 2683 |

0.11.6 대비 관측 차이는 5~15분 FPS +63.4% / CPU/tick -28.4%, 후반 FPS +65.2% / CPU/tick -29.3%였다. CPU 사용량은 두 구간 모두 약 17% 증가했다. 접속 완료 시점을 맞춘 비교도 같은 방향이었다.

CPU 코어 상당량은 Battle 프로세스의 user+kernel CPU 시간 증가량/벽시계 시간이며, CPU ms/tick은 같은 구간의 프로세스 CPU 시간/tick 증가량이다. Rayon worker만의 CPU 시간이나 물리 코어 점유율과는 다르다.

각 구현 1회이며 로컬 다른 프로세스를 격리하지 않았다. 시나리오·접속 수는 같지만 실제 상태와 행동까지 동일한 replay는 아니므로 확정 개선율이나 게임 결과 동등성을 보장하지 않는다. 목표 60 FPS도 지속 달성하지 못했다. 개별 프레임 지연 분포는 수집하지 않아 p95/p99를 주장하지 않는다.

원자료는 측정 워크스페이스의 `Deploy/Server/profiling/shipyard-chunk-vs-0116-2026-09-08_13-07-41`에 보존했다. `comparison.md`, `window-summary.json`, `intervals.csv`, `matrix.json`, 소스 해시·의존성 lock·실행별 120개 snapshot·executable/PDB를 포함한다. 원자료와 바이너리는 이 Git 커밋에 포함하지 않는다.

- chunk-planning 실행 바이너리 SHA256: `9D1CBD2EBFD9ECB0DA787C2E50B61092BC65671FFCFE852C17FA5D1B12964C0E`
- 0.11.6 실행 바이너리 SHA256: `26BA96054511C6C48338BD9626619CAC44BCEE9D93D87F78757F6DEEE1BDC7B5`

## 합성 비교

`examples/chunk_planning.rs`는 checksum까지 확인하는 독립 iterator 비교 도구다. 별도 임시 Cargo 패키지에서 같은 소스를 원본 커밋에 연결해 비교했다. 16 worker, 각 패턴 3,000회씩 5개 구간의 중앙값이다. 단위는 쿼리당 microseconds다.

| 패턴 | 원본 | 개선 |
|---|---:|---:|
| 65,536개 storage, 변경 없음 | 26.92 | 0.64 |
| 뒤쪽에 변경 1,024개 집중 | 30.21 | 21.04 |
| 같은 tail에 후보별 추가 연산 | 174.48 | 25.06 |
| 모든 chunk에 변경 1개씩 | 26.51 | 26.80 |
| 두 chunk마다 변경 1개 | 24.51 | 27.37 |
| 전체 변경 | 37.54 | 45.63 |
| 작은 64개 전체 변경 | 3.35 | 3.51 |
| tracking 없는 join | 29.59 | 31.91 |

로컬 다른 프로세스를 격리하지 않은 microbenchmark이며 반복 비교에서 일반 입력 수치는 변동했다. 빈 입력과 연산이 있는 sparse tail의 이득은 뚜렷했지만 조밀하거나 가벼운 입력의 개선은 보장하지 않는다. 위 합성 결과의 비율을 서버 FPS 개선율로 환산하면 안 되며, Battle 실측은 앞 절의 별도 비교를 참고한다.

```powershell
cargo run --release -p shipyard --example chunk_planning -- 16 3000
```
