# Tracking chunk 계획

기준은 Npixel-Eclipse/shipyard의 `origin/0.11.5`, 커밋 `d3dbd51435c225629564f3eda723291747aebda1`이다. 리뷰 브랜치는 `0.11.5-chunk-planning`이며, `0.11.6` 통합은 포함하지 않는다. 아래 Battle 비교를 위해 임시 적용한 의존성·설정·배포 바이너리는 측정 후 모두 복구했다. 별도 patch5 저장소도 그대로 유지했다.

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
- 0.11.6의 `max_chunks = (driver_len / 8).max(1)` 검사 예산은 아직 반영하지 않았다. 작은 driver와 큰 tracking storage를 join할 때 요약 생성 비용이 클 수 있다. 현재 요약은 captain 선택 전에 생성되므로, 추후 상한은 빈 입력 검사뿐 아니라 요약 생성 앞단에서 다뤄야 한다.
- reverse iteration에서도 tracking captain을 검사하도록 수정했다. WithId::next_back의 entity index와 WithId::fold의 OR source 전환도 회귀 테스트 범위에 맞춰 바로잡았다.
- OR의 병렬 지원과 기존 RawEntityIdAccess follow-up 분할 문제는 이번 범위에 포함하지 않았다. workload batch fast path나 Battle 시스템 코드는 변경하지 않았다.

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
