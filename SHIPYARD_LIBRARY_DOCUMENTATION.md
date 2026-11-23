# Shipyard ECS 라이브러리 상세 문서

## 목차
1. [개요](#개요)
2. [핵심 아키텍처](#핵심-아키텍처)
3. [주요 데이터 구조](#주요-데이터-구조)
4. [ECS 핵심 개념](#ecs-핵심-개념)
5. [고급 기능](#고급-기능)
6. [모듈 구조](#모듈-구조)
7. [사용 예제](#사용-예제)
8. [성능 최적화](#성능-최적화)

---

## 개요

### 라이브러리 정보
- **이름**: Shipyard
- **버전**: 0.9.0
- **유형**: Entity Component System (ECS)
- **라이선스**: MIT OR Apache-2.0
- **특징**: 사용성과 속도에 중점을 둔 Rust ECS 라이브러리

### 핵심 설계 철학
Shipyard는 베네치아 조선소(Venetian Arsenal)의 어셈블리 라인 개념에서 영감을 받았습니다. 조선소가 매일 완성된 배를 생산했듯이, Shipyard는 고도로 병렬화된 소프트웨어 프로세스 구축을 가능하게 합니다.

### 주요 특징
- ✅ **No-std 지원**: 표준 라이브러리 없이도 동작 가능
- ⚡ **고성능**: EnTT의 SparseSet 기반 구조 사용
- 🔄 **병렬 처리**: Rayon 기반 병렬 반복 및 워크로드 실행
- 🔒 **타입 안전성**: 컴파일 타임 빌림 검사
- 📊 **변경 추적**: 타임스탬프 기반 컴포넌트 변경 감지
- 🎯 **유연성**: 커스텀 스토리지 지원
- 🧵 **스레드 로컬**: `!Send`, `!Sync` 컴포넌트 지원

---

## 핵심 아키텍처

### 설계 원칙

```
World (최상위 컨테이너)
  │
  ├─ AllStorages (모든 스토리지 관리)
  │    ├─ Entities (엔티티 관리)
  │    └─ SparseSet<T> (컴포넌트 스토리지들)
  │
  └─ Scheduler (시스템 스케줄링)
       └─ Workloads (시스템 배치)
```

### 내부 가변성 패턴
Shipyard는 `AtomicRefCell`을 통한 런타임 빌림 검사를 사용합니다:
- 원자적 연산 기반 참조 카운팅
- 패닉 없는 오류 처리
- 스레드 안전성 보장

### 타입 소거 (Type Erasure)
- `SBox` (type-erased box)를 통해 이질적인 스토리지를 `HashMap`에 저장
- `StorageId`로 각 스토리지 타입 구별
- 런타임 타입 안전성 유지

---

## 주요 데이터 구조

### 1. EntityId

**위치**: `src/entity_id/mod.rs`

#### 구조
```rust
pub struct EntityId(NonZeroU64);

// 64비트 패킹 구조:
// [46비트 인덱스] [16비트 세대] [2비트 메타데이터]
```

#### 주요 특징
- **인덱스** (46비트): 최대 70조 개의 엔티티 지원
- **세대** (16비트): ABA 문제 방지, 65,534번까지 재사용 가능
- **메타데이터** (2비트): 내부 플래그용
- **NonZeroU64**: 메모리 최적화 (Option<EntityId>가 8바이트)

#### 주요 메서드
```rust
// 인덱스 추출
pub fn index(self) -> u64
pub fn uindex(self) -> usize

// 세대 추출
pub fn gen(self) -> u16

// 새 엔티티 생성
pub fn new(index: u64) -> Self
pub fn new_from_index_and_gen(index: u64, gen: u16) -> Self

// 버킷 계산 (sparse array 용)
fn bucket(self) -> usize
fn bucket_index(self) -> usize

// 죽은 엔티티 (null 엔티티로 사용)
pub fn dead() -> Self
```

#### 세대 관리
- 엔티티 삭제 시 세대 증가
- 최대 세대(65,534) 도달 시 영구 사용 불가
- 세대 불일치로 stale 참조 감지

---

### 2. SparseSet<T>

**위치**: `src/sparse_set/mod.rs`

#### 개념
Sparse Set은 희소 배열과 밀집 배열을 결합한 자료구조입니다:

```
Sparse Array (버킷화):
[bucket 0][bucket 1][bucket 2]...
    ↓          ↓          ↓
  [256B]     [256B]     [256B]

Dense Arrays (연속 메모리):
dense:  [EntityId, EntityId, EntityId, ...]
data:   [Component, Component, Component, ...]
```

#### 구조
```rust
pub struct SparseSet<T: Component> {
    // 핵심 데이터
    sparse: SparseArray<EntityId, BUCKET_SIZE>,  // 엔티티 → 인덱스 매핑
    dense: Vec<EntityId>,                         // 연속된 엔티티 ID
    data: Vec<T>,                                 // 연속된 컴포넌트 데이터

    // 추적 데이터
    last_insert: TrackingTimestamp,
    last_modified: TrackingTimestamp,
    insertion_data: Vec<TrackingTimestamp>,
    modification_data: Vec<TrackingTimestamp>,
    deletion_data: Vec<(EntityId, TrackingTimestamp, T)>,
    removal_data: Vec<(EntityId, TrackingTimestamp)>,

    // 추적 플래그
    is_tracking_insertion: bool,
    is_tracking_modification: bool,
    is_tracking_deletion: bool,
    is_tracking_removal: bool,

    // 콜백
    on_insertion: Option<Box<dyn FnMut(EntityId, &T) + Send + Sync>>,
    on_removal: Option<Box<dyn FnMut(EntityId, &T) + Send + Sync>>,

    // 복제 함수
    clone: Option<fn(&T) -> T>,
}
```

#### 성능 특성
- **삽입**: O(1) - dense/data 배열 끝에 추가
- **삭제**: O(1) - swap_remove로 마지막 요소와 교환
- **조회**: O(1) - sparse 배열 인덱싱
- **반복**: O(n) - 캐시 친화적 (dense 배열 순회)

#### 버킷 시스템
```rust
const BUCKET_SIZE: usize = 256 / size_of::<EntityId>(); // 일반적으로 32

// 엔티티 인덱스를 버킷으로 분할
// 예: entity_index 100 -> bucket 3, bucket_index 4
bucket = entity_index / BUCKET_SIZE
bucket_index = entity_index % BUCKET_SIZE
```

**장점**:
- 메모리 효율: 사용하지 않는 버킷 할당 안 함
- 캐시 효율: 256바이트 = 4 캐시 라인
- 병렬화: 버킷 단위로 청크 분할 가능

#### 추적 시스템 통합
```rust
// 삽입 추적
if self.is_tracking_insertion {
    self.insertion_data[index] = current_timestamp;
}

// 수정 추적 (Mut<T> wrapper 통해 자동)
if self.is_tracking_modification {
    self.modification_data[index] = current_timestamp;
}

// 삭제 추적 (컴포넌트 저장)
if self.is_tracking_deletion {
    self.deletion_data.push((entity, timestamp, component));
}

// 제거 추적 (컴포넌트 미저장)
if self.is_tracking_removal {
    self.removal_data.push((entity, timestamp));
}
```

---

### 3. World

**위치**: `src/world.rs`

#### 구조
```rust
pub struct World {
    all_storages: AtomicRefCell<AllStorages>,  // 모든 데이터
    scheduler: AtomicRefCell<Scheduler>,        // 시스템 스케줄러
    counter: Arc<AtomicU64>,                    // 글로벌 추적 카운터
    #[cfg(feature = "parallel")]
    thread_pool: Option<rayon::ThreadPool>,     // 선택적 스레드 풀
}
```

#### 주요 역할
1. **최상위 컨테이너**: 모든 ECS 데이터 관리
2. **API 진입점**: 사용자 친화적 인터페이스 제공
3. **리소스 관리**: 스토리지 및 스케줄러 생명주기 관리

#### 핵심 메서드

**엔티티 관리**:
```rust
// 엔티티 생성
world.add_entity((Position { x: 0.0, y: 0.0 }, Health(100)));

// 여러 엔티티 생성
world.bulk_add_entity((0..1000).map(|i| (Position::default(),)));

// 엔티티 삭제
world.delete_entity(entity_id);
```

**스토리지 접근**:
```rust
// 빌림 (borrow)
let positions = world.borrow::<View<Position>>().unwrap();
let mut velocities = world.borrow::<ViewMut<Velocity>>().unwrap();

// 실행 (자동 빌림)
world.run(|positions: View<Position>| {
    // 시스템 코드
});
```

**유니크 스토리지** (싱글톤):
```rust
world.add_unique(GameConfig { difficulty: 5 });
world.run(|config: UniqueView<GameConfig>| {
    println!("Difficulty: {}", config.difficulty);
});
```

**워크로드 실행**:
```rust
// 워크로드 추가
world.add_workload("Physics")
    .with_system(apply_gravity)
    .with_system(update_velocity)
    .with_system(check_collisions)
    .build();

// 실행
world.run_workload("Physics").unwrap();
world.run_default(); // 기본 워크로드
```

---

### 4. AllStorages

**위치**: `src/all_storages/mod.rs`

#### 구조
```rust
pub struct AllStorages {
    storages: RwLock<ShipHashMap<StorageId, SBox>>,  // 타입 소거된 스토리지
    counter: Arc<AtomicU64>,                         // 추적 타임스탬프
    #[cfg(feature = "thread_local")]
    thread_id_generator: Arc<dyn Fn() -> u64 + Send + Sync>,
}
```

#### StorageId
```rust
pub enum StorageId {
    TypeId(TypeId),           // 일반 컴포넌트
    Unique(TypeId),           // 유니크 컴포넌트
    Custom(u64),              // 커스텀 스토리지
}
```

#### 타입 소거 (SBox)
```rust
// Storage trait을 구현한 모든 타입을 Box로 감쌈
type SBox = Box<dyn Storage>;

pub trait Storage: Send + Sync {
    fn delete(&mut self, entity: EntityId, current: TrackingTimestamp);
    fn clear(&mut self, current: TrackingTimestamp);
    fn memory_usage(&self) -> StorageMemoryUsage;
    // ...
}
```

#### 주요 기능

**스토리지 등록**:
```rust
// 컴포넌트 스토리지 자동 생성
all_storages.register::<Position>();

// 커스텀 스토리지 등록
all_storages.register_custom_storage(CustomStorageId, custom_storage);
```

**엔티티 조작**:
```rust
// 엔티티 생성
let entity = all_storages.add_entity((Position::default(), Velocity::default()));

// 컴포넌트 추가
all_storages.add_component(entity, Health(100));

// 엔티티 삭제 (모든 컴포넌트 제거)
all_storages.delete_entity(entity);
```

**빌림 시스템**:
```rust
// View<T> 생성 시 내부적으로 발생
let storage = all_storages.borrow::<SparseSet<Position>>()?;
```

---

### 5. Entities

**위치**: `src/entities/mod.rs`

#### 구조
```rust
pub struct Entities {
    data: Vec<EntityId>,                   // 모든 엔티티 (살아있음 + 제거됨 + 죽음)
    list: Option<(usize, usize)>,          // 재사용 가능한 제거된 엔티티 연결 리스트
    #[cfg(feature = "thread_local")]
    on_deletion: Option<Box<dyn FnMut(EntityId)>>,  // 삭제 콜백
}
```

#### 엔티티 생명주기

**1. 생성**:
```rust
pub fn generate(&mut self) -> EntityId {
    if let Some((start, end)) = self.list {
        // 재사용 가능한 엔티티가 있으면 재사용
        let entity_id = self.data[start];

        if start == end {
            self.list = None;  // 리스트 비움
        } else {
            self.list = Some((entity_id.uindex(), end));  // 다음으로 이동
        }

        entity_id
    } else {
        // 새 엔티티 생성
        let index = self.data.len() as u64;
        let entity_id = EntityId::new(index);
        self.data.push(entity_id);
        entity_id
    }
}
```

**2. 삭제 및 재사용**:
```rust
pub fn delete(&mut self, entity_id: EntityId) -> Result<(), EntityId> {
    let actual = self.data[entity_id.uindex()];

    if actual == entity_id {
        // 세대 증가
        self.data[entity_id.uindex()].bump_gen()?;

        // 재사용 리스트에 추가
        if let Some((start, end)) = self.list {
            self.data[end].set_index(entity_id.index());
            self.list = Some((start, entity_id.uindex()));
        } else {
            self.list = Some((entity_id.uindex(), entity_id.uindex()));
        }

        Ok(())
    } else {
        Err(entity_id)  // 이미 삭제됨
    }
}
```

#### 세대 관리의 중요성
```rust
// 엔티티 A 생성: EntityId { index: 0, gen: 0 }
let entity_a = entities.generate();

// 엔티티 A 삭제 → gen 증가
entities.delete(entity_a); // EntityId { index: 0, gen: 1 }

// 엔티티 B 생성 (인덱스 0 재사용)
let entity_b = entities.generate(); // EntityId { index: 0, gen: 1 }

// entity_a는 이제 유효하지 않음 (세대 불일치)
// entity_a.gen() == 0 != entity_b.gen() == 1
```

---

## ECS 핵심 개념

### 1. Component (컴포넌트)

**위치**: `src/component.rs`

#### 정의
컴포넌트는 World에 저장할 수 있는 데이터 타입입니다.

```rust
pub trait Component: Sized + 'static {
    type Tracking: Tracking = track::Untracked;
}

// 자동 구현 (proc macro)
#[derive(Component)]
struct Position {
    x: f32,
    y: f32,
}

// 수동 구현 (추적 설정)
impl Component for Health {
    type Tracking = track::Insertion;  // 삽입만 추적
}
```

#### 추적 옵션
```rust
use shipyard::track;

// 추적 없음 (기본값)
type Tracking = track::Untracked;

// 삽입 추적
type Tracking = track::Insertion;

// 수정 추적
type Tracking = track::Modification;

// 삭제 추적 (컴포넌트 데이터 보존)
type Tracking = track::Deletion;

// 제거 추적 (데이터 보존 안 함)
type Tracking = track::Removal;

// 조합
type Tracking = track::InsertionModification;
type Tracking = track::All;  // 모든 추적
```

#### 제약 조건
- `Send + Sync`: 기본적으로 스레드 안전해야 함
- `'static`: 생명주기 제약 없음

**스레드 로컬 컴포넌트** (thread_local 기능):
```rust
// !Send
impl Component for NonSendComponent {
    type Tracking = track::Untracked;
}

// 사용
world.run(|components: NonSend<View<NonSendComponent>>| {
    // ...
});
```

---

### 2. System (시스템)

**위치**: `src/system/mod.rs`

#### 정의
시스템은 World에서 실행할 수 있는 함수입니다.

```rust
pub trait System {
    fn run(world: &World) -> Result<(), error::Run>;
}

// 자동 구현 (최대 10개 매개변수, extended_tuple로 32개)
impl<F, R, T1, T2, ...> System for F
where
    F: Fn(T1, T2, ...) -> R,
    T1: WorldBorrow,
    T2: WorldBorrow,
    ...
```

#### 기본 시스템 예제
```rust
fn apply_gravity(mut positions: ViewMut<Position>, dt: UniqueView<DeltaTime>) {
    for pos in (&mut positions).iter() {
        pos.y -= 9.8 * dt.0;
    }
}

fn check_collisions(positions: View<Position>, hitboxes: View<Hitbox>) {
    for (pos, hitbox) in (&positions, &hitboxes).iter() {
        // 충돌 검사
    }
}
```

#### 시스템 매개변수 (WorldBorrow)

**View Types**:
- `View<T>`: 컴포넌트 읽기 (`&T`)
- `ViewMut<T>`: 컴포넌트 쓰기 (`&mut T`)
- `EntitiesView`: 엔티티 읽기
- `EntitiesViewMut`: 엔티티 생성/삭제
- `UniqueView<T>`: 유니크 스토리지 읽기
- `UniqueViewMut<T>`: 유니크 스토리지 쓰기
- `AllStoragesView`: 모든 스토리지 읽기
- `AllStoragesViewMut`: 모든 스토리지 쓰기 (독점)

**조합 가능**:
```rust
fn complex_system(
    positions: View<Position>,
    mut velocities: ViewMut<Velocity>,
    entities: EntitiesView,
    config: UniqueView<GameConfig>,
) {
    // 시스템 로직
}
```

#### 반환 값
시스템은 값을 반환할 수 있습니다:
```rust
fn count_entities(positions: View<Position>) -> usize {
    positions.len()
}

let count = world.run(count_entities);
println!("Total entities: {}", count);
```

---

### 3. View (뷰)

**위치**: `src/views/`

#### View<T> - 읽기 전용 뷰

```rust
pub struct View<'a, T: Component> {
    sparse_set: Ref<'a, SparseSet<T>>,
    last_insertion: TrackingTimestamp,
    last_modification: TrackingTimestamp,
    current: TrackingTimestamp,
}
```

**주요 메서드**:
```rust
// 컴포넌트 조회
view.get(entity) -> Option<&T>
view.contains(entity) -> bool

// 반복
for component in view.iter() { }
for (entity, component) in view.iter().with_id() { }

// 추적
view.inserted()           // Inserted<T> iterator
view.modified()           // Modified<T> iterator
view.inserted_or_modified() // InsertedOrModified<T> iterator
```

#### ViewMut<T> - 읽기/쓰기 뷰

```rust
pub struct ViewMut<'a, T: Component> {
    sparse_set: RefMut<'a, SparseSet<T>>,
    last_insertion: TrackingTimestamp,
    last_modification: TrackingTimestamp,
    last_deletion: TrackingTimestamp,
    last_removal: TrackingTimestamp,
    current: TrackingTimestamp,
}
```

**추가 메서드**:
```rust
// 가변 접근
view_mut.get(entity) -> Option<Mut<T>>  // 자동 수정 추적
(&mut view_mut).iter() -> Iterator<Item = Mut<T>>

// 컴포넌트 추가/제거
view_mut.add_component(entity, component);
view_mut.remove(entity) -> Option<T>;
view_mut.delete(entity);  // 추적 데이터에 저장

// 예약 (메모리 사전 할당)
view_mut.reserve(1000);
```

#### Mut<T> - 수정 추적 래퍼

```rust
pub struct Mut<'a, T> {
    data: &'a mut T,
    flag: &'a mut TrackingTimestamp,
    current: TrackingTimestamp,
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { self.data }
}

impl<T> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        *self.flag = self.current;  // 수정 자동 기록
        self.data
    }
}
```

**사용 예**:
```rust
for mut pos in (&mut positions).iter() {
    pos.x += 1.0;  // DerefMut 호출 → 수정 추적
}
```

---

## 고급 기능

### 1. 반복 시스템 (Shiperator)

**위치**: `src/iter/mod.rs`

#### 설계 개념
Shipyard의 커스텀 반복자 "Shiperator"는 두 가지 모드를 지원합니다:

**Captain 모드** (정확한 크기):
- 모든 스토리지가 동일한 엔티티 집합을 가질 때
- 엔티티 존재 확인 불필요
- 빠른 반복

**Sailor 모드** (희소):
- 스토리지마다 다른 엔티티 집합
- 각 엔티티 존재 확인 필요
- 유연한 반복

#### 핵심 구조
```rust
pub struct Shiperator<S> {
    shiperator: S,                    // Captain 또는 Sailor
    entities: RawEntityIdAccess,      // 엔티티 ID 접근
    is_exact_sized: bool,             // Captain 모드 여부
    start: usize,                     // 현재 위치
    end: usize,                       // 현재 슬라이스 끝
}
```

#### 반복 알고리즘
```rust
fn next(&mut self) -> Option<Self::Item> {
    loop {
        if self.start == self.end {
            // 다음 버킷으로 이동
            if let Some(new_end) = self.entities.next_slice() {
                self.start = 0;
                self.end = new_end;
                self.shiperator.next_slice();
            } else {
                return None;  // 반복 종료
            }
        }

        let current = self.start;
        self.start += 1;

        if self.is_exact_sized {
            // Captain: 직접 데이터 가져오기
            return unsafe { Some(self.shiperator.get_captain_data(current)) };
        } else {
            // Sailor: 엔티티 존재 확인
            let entity_id = unsafe { self.entities.get(current) };
            if let Some(indices) = self.shiperator.indices_of(entity_id, current) {
                return unsafe { Some(self.shiperator.get_sailor_data(indices)) };
            }
        }
    }
}
```

#### 기본 반복
```rust
// 단일 컴포넌트
for pos in positions.iter() {
    println!("{:?}", pos);
}

// 여러 컴포넌트 (튜플)
for (pos, vel) in (&positions, &velocities).iter() {
    // ...
}

// 엔티티 ID 포함
for (id, pos, vel) in (&positions, &velocities).iter().with_id() {
    println!("Entity {:?}: pos={:?}, vel={:?}", id, pos, vel);
}
```

#### 필터링
```rust
// 수동 필터
for pos in positions.iter().filter(|p| p.x > 0.0) {
    // ...
}

// Optional (있으면 Some, 없으면 None)
use shipyard::Optional;
for (pos, opt_vel) in (&positions, Optional(&velocities)).iter() {
    if let Some(vel) = opt_vel {
        // velocity가 있는 엔티티만
    }
}

// Not (컴포넌트가 없는 엔티티만)
use shipyard::Not;
for pos in (&positions, !&tags).iter() {
    // Tag가 없는 position만
}
```

#### 병렬 반복 (parallel 기능)
```rust
use rayon::prelude::*;

(&mut positions).par_iter().for_each(|mut pos| {
    pos.x *= 2.0;
});

// 여러 컴포넌트
(&mut positions, &velocities).par_iter().for_each(|(mut pos, vel)| {
    pos.x += vel.x;
    pos.y += vel.y;
});
```

**병렬화 전략**:
- 버킷 단위로 청크 분할
- Rayon의 work-stealing 스케줄러 사용
- 데이터 경쟁 없음 (각 청크 독립적)

---

### 2. 추적 시스템 (Tracking)

**위치**: `src/tracking.rs`, `src/track/`

#### 타임스탬프 기반 추적
```rust
pub struct TrackingTimestamp(u64);

impl TrackingTimestamp {
    // 타임스탬프가 [last, current] 범위에 있는지 확인
    pub fn is_within(self, last: TrackingTimestamp, current: TrackingTimestamp) -> bool {
        last.0 < self.0 && self.0 <= current.0
    }
}
```

#### 추적 작동 방식

**1. 글로벌 카운터**:
```rust
// World 생성 시
counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(1));

// 각 시스템 실행 시
let current = counter.fetch_add(1, Ordering::Relaxed);
```

**2. 컴포넌트 수정 시**:
```rust
// Mut<T>의 DerefMut
impl<T> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        *self.modification_flag = self.current_timestamp;
        self.data
    }
}
```

**3. 추적 쿼리**:
```rust
fn track_changes(positions: View<Position, track::Modification>) {
    // 마지막 실행 이후 수정된 컴포넌트만
    for (id, pos) in positions.modified().with_id() {
        println!("Entity {:?} moved to {:?}", id, pos);
    }
}
```

#### 추적 타입

**삽입 추적** (`track::Insertion`):
```rust
impl Component for NewComponent {
    type Tracking = track::Insertion;
}

fn handle_new(components: View<NewComponent>) {
    for comp in components.inserted() {
        println!("New component added!");
    }
}
```

**수정 추적** (`track::Modification`):
```rust
impl Component for Transform {
    type Tracking = track::Modification;
}

fn handle_modified(transforms: View<Transform>) {
    for transform in transforms.modified() {
        // 변경된 변환만 처리
        update_render_system(transform);
    }
}
```

**삭제 추적** (`track::Deletion`):
```rust
impl Component for Important {
    type Tracking = track::Deletion;
}

fn handle_deleted(mut components: ViewMut<Important>) {
    for (id, old_value) in components.deleted() {
        println!("Entity {:?} lost component: {:?}", id, old_value);
    }

    // 추적 데이터 정리
    components.clear_all_deleted();
}
```

**제거 추적** (`track::Removal`):
```rust
// Deletion과 유사하지만 컴포넌트 데이터 보존 안 함
impl Component for Tag {
    type Tracking = track::Removal;
}
```

#### 추적 조합
```rust
// 삽입 및 수정
type Tracking = track::InsertionModification;

// 삽입, 수정, 삭제, 제거 모두
type Tracking = track::All;

// 사용
fn handle_all(components: View<Component>) {
    for comp in components.inserted_or_modified() {
        // 새로 추가되거나 수정된 컴포넌트
    }
}
```

#### 추적 정리
```rust
// 특정 타임스탬프 이전 데이터 삭제
components.clear_all_inserted_older_than(old_timestamp);
components.clear_all_modified_older_than(old_timestamp);
components.clear_all_deleted();
components.clear_all_removed();
```

---

### 3. 워크로드 시스템 (Workload)

**위치**: `src/scheduler/`

#### 개념
워크로드는 시스템의 컬렉션으로, 자동으로 병렬화됩니다.

```rust
// 워크로드 빌더
world.add_workload("Physics")
    .with_system(apply_gravity)
    .with_system(update_velocity)
    .with_system(apply_velocity)
    .build();
```

#### Batches - 자동 병렬화
```rust
pub struct Batches {
    parallel: Vec<(Option<usize>, Vec<usize>)>,     // 병렬 실행 가능한 시스템들
    parallel_run_if: Vec<(Option<usize>, Vec<usize>)>,
    sequential: Vec<usize>,                          // 순차 실행 필요한 시스템
    sequential_run_if: Vec<...>,
    run_if: Option<Box<dyn WorkloadRunIfFn>>,       // 워크로드 조건부 실행
}
```

**스케줄링 알고리즘**:
1. 시스템의 빌림 요구사항 분석
2. 충돌하지 않는 시스템들을 같은 배치에 그룹화
3. 배치 내에서 병렬 실행
4. 배치 간에는 순차 실행

**예제**:
```rust
// 이 시스템들은...
fn system_a(a: ViewMut<A>) { }           // A를 변경
fn system_b(b: ViewMut<B>) { }           // B를 변경
fn system_c(a: View<A>, b: View<B>) { }  // A, B를 읽음

// 다음과 같이 스케줄됨:
// Batch 1 (병렬): [system_a, system_b]  // A와 B는 충돌 없음
// Batch 2 (순차): [system_c]            // A와 B를 읽으므로 이전 배치 후 실행
```

#### 시스템 수정자 (SystemModificator)
```rust
world.add_workload("Game")
    .with_system(physics_system)
    .with_system(render_system.before(physics_system))  // 순서 지정
    .with_system(debug_system.run_if(is_debug_mode))    // 조건부 실행
    .with_system(system.tag("important"))               // 태그 지정
    .build();
```

**메서드**:
- `.before(label)`: 특정 시스템 전에 실행
- `.after(label)`: 특정 시스템 후에 실행
- `.before_all()`: 모든 시스템 전에 실행
- `.after_all()`: 모든 시스템 후에 실행
- `.run_if(condition)`: 조건부 실행
- `.tag(label)`: 시스템에 라벨 부여

#### 조건부 실행 (run_if)
```rust
fn is_paused(paused: UniqueView<Paused>) -> bool {
    paused.0
}

world.add_workload("Update")
    .with_system(update_physics.run_if(|| !is_paused))
    .with_system(update_ai.run_if(|paused: UniqueView<Paused>| !paused.0))
    .build();
```

#### 워크로드 실행
```rust
// 이름으로 실행
world.run_workload("Physics")?;

// 기본 워크로드 실행
world.run_default();

// 기본 워크로드 설정
world.set_default_workload("Game")?;
```

#### 워크로드 정보 조회
```rust
let info = world.workloads_info();

for (name, workload_info) in info {
    println!("Workload: {}", name);

    for batch in &workload_info.batch_info {
        println!("  Batch:");
        for system in &batch.systems {
            println!("    - {}", system.name);
        }
    }
}
```

---

### 4. 커스텀 스토리지

**위치**: `src/storage/mod.rs`

#### Storage Trait
```rust
pub trait Storage: Send + Sync {
    fn delete(&mut self, entity: EntityId, current: TrackingTimestamp);
    fn clear(&mut self, current: TrackingTimestamp);
    fn memory_usage(&self) -> StorageMemoryUsage;
    fn sparse_array_len(&self) -> Option<usize> { None }
    fn is_empty(&self) -> bool;
    // ...
}
```

#### 커스텀 스토리지 구현 예제
```rust
use shipyard::{Storage, SparseSet, Component, EntityId};
use std::collections::HashMap;

// HashMap 기반 스토리지 (희소 데이터에 적합)
pub struct HashMapStorage<T> {
    data: HashMap<EntityId, T>,
    // 추적 데이터 등...
}

impl<T: Component> Storage for HashMapStorage<T> {
    fn delete(&mut self, entity: EntityId, _current: TrackingTimestamp) {
        self.data.remove(&entity);
    }

    fn clear(&mut self, _current: TrackingTimestamp) {
        self.data.clear();
    }

    fn memory_usage(&self) -> StorageMemoryUsage {
        StorageMemoryUsage {
            storage_name: "HashMapStorage",
            allocated_bytes: self.data.capacity() * std::mem::size_of::<(EntityId, T)>(),
            used_bytes: self.data.len() * std::mem::size_of::<(EntityId, T)>(),
            component_count: self.data.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
```

#### 스토리지 등록
```rust
world.add_custom_storage::<MyComponent, HashMapStorage<MyComponent>>(
    HashMapStorage::new()
)?;
```

#### 사용 사례
- **HashMap**: 매우 희소한 컴포넌트 (< 1% 엔티티)
- **Vec**: 거의 모든 엔티티가 가진 컴포넌트
- **SparseSet**: 기본값, 대부분의 경우에 최적
- **커스텀**: 특수한 데이터 구조 (예: 공간 파티셔닝)

---

### 5. 스레드 로컬 지원 (thread_local 기능)

**위치**: `src/borrow/`

#### 개념
일부 타입은 `Send`나 `Sync`를 구현하지 않습니다:
- `Rc<T>`: !Send, !Sync
- `Cell<T>`: !Sync
- Platform-specific 핸들

#### 래퍼 타입
```rust
// !Send (다른 스레드로 이동 불가)
pub struct NonSend<T> { /* ... */ }

// !Sync (스레드 간 공유 불가)
pub struct NonSync<T> { /* ... */ }

// !Send + !Sync
pub struct NonSendSync<T> { /* ... */ }
```

#### 사용 예제
```rust
use std::rc::Rc;

#[derive(Component)]
struct RcComponent {
    data: Rc<String>,
}

// NonSend로 감싸서 사용
fn system(components: NonSend<View<RcComponent>>) {
    for comp in components.iter() {
        println!("{}", comp.data);
    }
}

// ViewMut도 동일
fn system_mut(mut components: NonSend<ViewMut<RcComponent>>) {
    for mut comp in (&mut *components).iter() {
        // ...
    }
}
```

#### 제약사항
- NonSend 컴포넌트는 생성된 스레드에서만 접근 가능
- 병렬 시스템에서 사용 불가
- 워크로드 스케줄링 시 순차 실행으로 제한

---

## 모듈 구조

### 전체 모듈 맵

```
shipyard/
├── src/
│   ├── lib.rs                          # 공개 API, 기능 플래그
│   │
│   ├── entity_id/                      # 엔티티 식별자
│   │   ├── mod.rs                      # EntityId 구현
│   │   └── serde.rs                    # Serialization 지원
│   │
│   ├── component.rs                    # Component, Unique trait
│   │
│   ├── sparse_set/                     # 기본 스토리지
│   │   ├── mod.rs                      # SparseSet 핵심
│   │   ├── sparse_array.rs             # 버킷화된 희소 배열
│   │   ├── add_component.rs            # 컴포넌트 추가
│   │   ├── bulk_add_entity.rs          # 벌크 엔티티 생성
│   │   ├── delete.rs                   # 컴포넌트 삭제
│   │   ├── remove.rs                   # 컴포넌트 제거
│   │   ├── drain.rs                    # Drain iterator
│   │   ├── window.rs                   # 로우 데이터 접근
│   │   └── memory_usage.rs             # 메모리 사용량
│   │
│   ├── all_storages/                   # 중앙 스토리지 관리
│   │   ├── mod.rs                      # AllStorages 핵심
│   │   ├── custom_storage.rs           # 커스텀 스토리지
│   │   └── retain.rs                   # 엔티티/컴포넌트 필터링
│   │
│   ├── entities/                       # 엔티티 관리
│   │   ├── mod.rs                      # Entities 구현
│   │   └── ...
│   │
│   ├── world/                          # World 구현
│   │   ├── builder.rs                  # WorldBuilder
│   │   └── run_batches.rs              # 배치 실행
│   ├── world.rs                        # World 공개 API
│   │
│   ├── views/                          # 스토리지 뷰
│   │   ├── view.rs                     # View<T>
│   │   ├── view_mut.rs                 # ViewMut<T>
│   │   ├── unique_view.rs              # UniqueView<T>
│   │   ├── entities.rs                 # EntitiesView
│   │   └── all_storages.rs             # AllStoragesView
│   │
│   ├── iter/                           # 반복 시스템
│   │   ├── mod.rs                      # Shiperator 핵심
│   │   ├── captain.rs                  # 정확한 크기 반복
│   │   ├── sailor.rs                   # 희소 반복
│   │   ├── parallel.rs                 # 병렬 반복
│   │   ├── mixed.rs                    # 혼합 반복
│   │   ├── with_id.rs                  # EntityId 포함 반복
│   │   └── into_shiperator.rs          # IntoIter trait
│   │
│   ├── tracking.rs                     # 추적 시스템
│   ├── tracking/                       # 추적 반복자
│   │   └── iterator_wrapper.rs         # Inserted, Modified 등
│   │
│   ├── track/                          # 추적 타입
│   │   ├── insertion.rs
│   │   ├── modification.rs
│   │   ├── deletion.rs
│   │   ├── removal.rs
│   │   └── ...                         # 조합 타입들
│   │
│   ├── scheduler/                      # 시스템 스케줄링
│   │   ├── mod.rs                      # Scheduler, Batches
│   │   ├── workload.rs                 # Workload 구현
│   │   ├── system.rs                   # WorkloadSystem
│   │   ├── into_workload.rs            # 워크로드 빌더
│   │   ├── into_workload_system.rs     # 시스템 추가
│   │   ├── system_modificator.rs       # 시스템 설정
│   │   ├── workload_modificator.rs     # 워크로드 설정
│   │   ├── label.rs                    # Label trait
│   │   └── info.rs                     # 워크로드 정보
│   │
│   ├── system/                         # 시스템 trait
│   │   └── mod.rs
│   │
│   ├── storage/                        # 추상 스토리지
│   │   ├── mod.rs                      # Storage trait
│   │   └── storage_id.rs               # StorageId
│   │
│   ├── borrow/                         # 빌림 시스템
│   │   ├── mod.rs                      # WorldBorrow trait
│   │   ├── non_send.rs                 # NonSend wrapper
│   │   ├── non_sync.rs                 # NonSync wrapper
│   │   └── non_send_sync.rs            # NonSendSync wrapper
│   │
│   ├── atomic_refcell/                 # 내부 가변성
│   │   ├── mod.rs                      # AtomicRefCell
│   │   ├── borrow.rs                   # Ref wrapper
│   │   └── borrow_mut.rs               # RefMut wrapper
│   │
│   ├── error.rs                        # 에러 타입들
│   ├── memory_usage.rs                 # 메모리 인트로스펙션
│   ├── type_id.rs                      # 타입 ID 관리
│   │
│   ├── get.rs                          # Get trait
│   ├── get_component.rs                # GetComponent trait
│   ├── get_unique.rs                   # GetUnique trait
│   │
│   ├── add_component.rs                # AddComponent trait
│   ├── add_distinct_component.rs       # AddDistinctComponent
│   ├── add_entity.rs                   # AddEntity trait
│   │
│   ├── delete.rs                       # Delete trait
│   ├── remove.rs                       # Remove trait
│   ├── contains.rs                     # Contains trait
│   │
│   ├── mut.rs                          # Mut<T> wrapper
│   ├── not.rs                          # Not query
│   ├── optional.rs                     # Optional query
│   ├── or.rs                           # Or query
│   │
│   └── unique/                         # Unique 스토리지
│       └── mod.rs
│
├── shipyard_proc/                      # Procedural macros
│   ├── src/
│   │   ├── lib.rs
│   │   ├── component.rs                # #[derive(Component)]
│   │   ├── borrow.rs                   # #[derive(Borrow)]
│   │   └── ...
│   └── Cargo.toml
│
├── tests/                              # 통합 테스트
├── guide/                              # 사용자 가이드
├── bunny_demo/                         # 데모 (벤치마크)
├── square_eater/                       # 게임 예제
└── visualizer/                         # 시각화 도구
```

### 주요 모듈 책임

**데이터 계층**:
- `entity_id`: 엔티티 식별 및 생명주기
- `sparse_set`: 컴포넌트 스토리지
- `all_storages`: 스토리지 관리
- `entities`: 엔티티 풀 관리

**접근 계층**:
- `views`: 안전한 스토리지 접근
- `borrow`: 빌림 규칙 및 스레드 안전성
- `atomic_refcell`: 런타임 빌림 검사

**쿼리 계층**:
- `iter`: 반복 시스템
- `tracking`: 변경 추적
- `get`, `contains`: 개별 접근

**실행 계층**:
- `world`: 최상위 API
- `system`: 시스템 정의
- `scheduler`: 워크로드 스케줄링

**유틸리티**:
- `error`: 에러 처리
- `memory_usage`: 메모리 프로파일링
- `type_id`: 타입 시스템

---

## 사용 예제

### 1. 기본 사용 - 위치와 속도

```rust
use shipyard::{Component, IntoIter, View, ViewMut, World};

#[derive(Component, Debug)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Component, Debug)]
struct Velocity {
    x: f32,
    y: f32,
}

fn main() {
    let mut world = World::new();

    // 엔티티 생성
    for i in 0..10 {
        world.add_entity((
            Position { x: i as f32, y: 0.0 },
            Velocity { x: 1.0, y: 0.5 },
        ));
    }

    // 시스템 실행
    world.run(update_positions);
    world.run(print_positions);
}

fn update_positions(mut positions: ViewMut<Position>, velocities: View<Velocity>) {
    for (pos, vel) in (&mut positions, &velocities).iter() {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}

fn print_positions(positions: View<Position>) {
    for (id, pos) in positions.iter().with_id() {
        println!("{:?}: {:?}", id, pos);
    }
}
```

### 2. 추적 사용 - 변경 감지

```rust
use shipyard::{Component, View, ViewMut, World, track};

#[derive(Component)]
struct Health {
    current: u32,
    max: u32,
}

impl Component for Health {
    type Tracking = track::Modification;
}

fn damage_system(mut healths: ViewMut<Health>) {
    // 일부 엔티티에 데미지
    for (i, mut health) in (&mut healths).iter().enumerate() {
        if i % 2 == 0 {
            health.current = health.current.saturating_sub(10);
        }
    }
}

fn health_ui_system(healths: View<Health>) {
    // 변경된 체력만 UI 업데이트
    for (id, health) in healths.modified().with_id() {
        println!("Update UI for {:?}: {}/{}", id, health.current, health.max);
    }
}

fn main() {
    let mut world = World::new();

    for _ in 0..5 {
        world.add_entity((Health { current: 100, max: 100 },));
    }

    world.run(damage_system);
    world.run(health_ui_system);  // 변경된 것만 처리
}
```

### 3. 워크로드 - 물리 시뮬레이션

```rust
use shipyard::{Component, IntoIter, Unique, UniqueView, UniqueViewMut, View, ViewMut, World};

#[derive(Component)]
struct Position { x: f32, y: f32 }

#[derive(Component)]
struct Velocity { x: f32, y: f32 }

#[derive(Component)]
struct Mass(f32);

#[derive(Unique)]
struct DeltaTime(f32);

#[derive(Unique)]
struct Gravity(f32);

fn apply_gravity(mut velocities: ViewMut<Velocity>, gravity: UniqueView<Gravity>, dt: UniqueView<DeltaTime>) {
    for mut vel in (&mut velocities).iter() {
        vel.y -= gravity.0 * dt.0;
    }
}

fn apply_drag(mut velocities: ViewMut<Velocity>) {
    for mut vel in (&mut velocities).iter() {
        vel.x *= 0.99;
        vel.y *= 0.99;
    }
}

fn integrate(mut positions: ViewMut<Position>, velocities: View<Velocity>, dt: UniqueView<DeltaTime>) {
    for (mut pos, vel) in (&mut positions, &velocities).iter() {
        pos.x += vel.x * dt.0;
        pos.y += vel.y * dt.0;
    }
}

fn check_bounds(mut positions: ViewMut<Position>, mut velocities: ViewMut<Velocity>) {
    for (mut pos, mut vel) in (&mut positions, &mut velocities).iter() {
        if pos.y < 0.0 {
            pos.y = 0.0;
            vel.y = -vel.y * 0.8;  // 반발
        }
    }
}

fn main() {
    let mut world = World::new();

    // 글로벌 설정
    world.add_unique(DeltaTime(1.0 / 60.0));
    world.add_unique(Gravity(9.8));

    // 워크로드 추가
    world.add_workload("Physics")
        .with_system(apply_gravity)
        .with_system(apply_drag)
        .with_system(integrate)
        .with_system(check_bounds)
        .build();

    // 엔티티 생성
    for i in 0..100 {
        world.add_entity((
            Position { x: i as f32 * 10.0, y: 100.0 },
            Velocity { x: 0.0, y: 0.0 },
            Mass(1.0),
        ));
    }

    // 시뮬레이션 루프
    for frame in 0..1000 {
        world.run_workload("Physics").unwrap();

        if frame % 60 == 0 {
            world.run(|positions: View<Position>| {
                println!("Frame {}: {} entities", frame, positions.len());
            });
        }
    }
}
```

### 4. 병렬 처리 - 대규모 연산

```rust
use shipyard::{Component, ViewMut, World};
use rayon::prelude::*;

#[derive(Component)]
struct ComplexData {
    values: Vec<f32>,
}

fn expensive_computation(mut data: ViewMut<ComplexData>) {
    // 병렬 반복
    (&mut data).par_iter().for_each(|mut comp| {
        // 각 엔티티에 대해 무거운 연산
        for value in &mut comp.values {
            *value = (*value * 2.0).sin().cos().sqrt();
        }
    });
}

fn main() {
    let mut world = World::new();

    // 많은 엔티티 생성
    for _ in 0..10_000 {
        world.add_entity((
            ComplexData {
                values: vec![1.0; 1000],
            },
        ));
    }

    // 병렬 실행
    world.run(expensive_computation);
}
```

### 5. 커스텀 쿼리 - Optional과 Not

```rust
use shipyard::{Component, EntityId, IntoIter, View, World, Not, Optional};

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Enemy;

#[derive(Component)]
struct Health(u32);

#[derive(Component)]
struct Shield(u32);

fn process_entities(
    entities: View<EntityId>,
    healths: View<Health>,
    shields: View<Shield>,
    players: View<Player>,
) {
    // Shield가 있으면 Some, 없으면 None
    for (id, health, opt_shield) in (&entities, &healths, Optional(&shields)).iter() {
        match opt_shield {
            Some(shield) => println!("{:?}: Health={}, Shield={}", id, health.0, shield.0),
            None => println!("{:?}: Health={}, No shield", id, health.0),
        }
    }

    // Player가 아닌 엔티티만 (적들)
    for (id, health) in (&entities, &healths, !&players).iter() {
        println!("Enemy {:?}: Health={}", id, health.0);
    }
}

fn main() {
    let mut world = World::new();

    // 플레이어 (Shield 있음)
    world.add_entity((Player, Health(100), Shield(50)));

    // 적들 (일부는 Shield 있음)
    world.add_entity((Enemy, Health(50)));
    world.add_entity((Enemy, Health(80), Shield(20)));
    world.add_entity((Enemy, Health(30)));

    world.run(process_entities);
}
```

### 6. 엔티티 관리 - 생성과 삭제

```rust
use shipyard::{Component, EntitiesViewMut, EntityId, View, ViewMut, World};

#[derive(Component)]
struct Health(i32);

#[derive(Component)]
struct Bullet {
    lifetime: f32,
}

fn spawn_bullets(mut entities: EntitiesViewMut, mut bullets: ViewMut<Bullet>) {
    // 새 엔티티 생성
    for _ in 0..10 {
        entities.add_entity(
            &mut bullets,
            Bullet { lifetime: 5.0 },
        );
    }
}

fn update_bullets(
    mut entities: EntitiesViewMut,
    mut bullets: ViewMut<Bullet>,
) -> Vec<EntityId> {
    let mut to_delete = Vec::new();

    for (id, mut bullet) in (&mut bullets).iter().with_id() {
        bullet.lifetime -= 0.016;  // 60 FPS

        if bullet.lifetime <= 0.0 {
            to_delete.push(id);
        }
    }

    // 엔티티 삭제
    for id in &to_delete {
        entities.delete(*id);
    }

    to_delete
}

fn remove_low_health(
    mut entities: EntitiesViewMut,
    healths: View<Health>,
) {
    // 조건부 삭제
    for (id, health) in healths.iter().with_id() {
        if health.0 <= 0 {
            entities.delete(id);
        }
    }
}

fn main() {
    let mut world = World::new();

    world.run(spawn_bullets);

    for _ in 0..400 {  // ~6.6 seconds at 60 FPS
        let deleted = world.run(update_bullets);
        if !deleted.is_empty() {
            println!("Deleted {} bullets", deleted.len());
        }
    }
}
```

### 7. 유니크 스토리지 - 글로벌 상태

```rust
use shipyard::{Unique, UniqueView, UniqueViewMut, World};

#[derive(Unique)]
struct GameState {
    score: u32,
    level: u32,
    paused: bool,
}

#[derive(Unique)]
struct Settings {
    volume: f32,
    difficulty: u32,
}

fn increment_score(mut state: UniqueViewMut<GameState>) {
    state.score += 10;
}

fn check_level_up(mut state: UniqueViewMut<GameState>) {
    if state.score >= state.level * 100 {
        state.level += 1;
        println!("Level up! Now level {}", state.level);
    }
}

fn game_loop(state: UniqueView<GameState>, settings: UniqueView<Settings>) {
    if !state.paused {
        println!(
            "Playing: Level {}, Score {}, Difficulty {}",
            state.level, state.score, settings.difficulty
        );
    }
}

fn main() {
    let mut world = World::new();

    world.add_unique(GameState {
        score: 0,
        level: 1,
        paused: false,
    });

    world.add_unique(Settings {
        volume: 0.8,
        difficulty: 5,
    });

    for _ in 0..15 {
        world.run(increment_score);
        world.run(check_level_up);
        world.run(game_loop);
    }
}
```

---

## 성능 최적화

### 1. 메모리 레이아웃

#### SparseSet의 캐시 친화성
```
메모리 레이아웃:
[Entity 0 comp][Entity 1 comp][Entity 2 comp]... <- 연속 메모리

vs

HashMap:
[Entity 0 comp] -> 랜덤 메모리
[Entity 5 comp] -> 랜덤 메모리
[Entity 2 comp] -> 랜덤 메모리
```

**결과**: SparseSet은 순회 시 캐시 미스율이 낮습니다.

### 2. 병렬 처리 최적화

#### 버킷 기반 청킹
```rust
// 버킷 크기: 256 bytes = 4 cache lines
const BUCKET_SIZE: usize = 256 / size_of::<EntityId>();

// 병렬 반복 시 각 스레드가 버킷 단위로 작업
// -> 캐시 경합 최소화
// -> 로드 밸런싱 개선
```

#### 워크로드 자동 병렬화
```rust
// 이 시스템들은 자동으로 병렬 실행됨:
fn system_a(a: ViewMut<A>) { }
fn system_b(b: ViewMut<B>) { }
fn system_c(c: ViewMut<C>) { }

// Shipyard가 자동으로:
// 1. 의존성 분석
// 2. 병렬 배치 생성
// 3. Rayon으로 실행
```

### 3. 추적 오버헤드 최소화

#### 선택적 추적
```rust
// 추적 없음 (빠름)
impl Component for StaticData {
    type Tracking = track::Untracked;
}

// 필요한 것만 추적
impl Component for Transform {
    type Tracking = track::Modification;  // 삽입은 추적 안 함
}
```

#### 추적 데이터 정리
```rust
// 오래된 추적 데이터 정리
fn cleanup_tracking(mut positions: ViewMut<Position>) {
    let old_timestamp = /* ... */;
    positions.clear_all_modified_older_than(old_timestamp);
}
```

### 4. 메모리 사전 할당

```rust
// 엔티티 대량 생성 전
world.bulk_reserve::<(Position, Velocity)>(10_000);

// 컴포넌트 추가 전
world.run(|mut positions: ViewMut<Position>| {
    positions.reserve(1000);
});
```

### 5. 반복 최적화

#### Captain vs Sailor
```rust
// Captain (빠름): 모든 엔티티가 Position과 Velocity를 가진 경우
for (pos, vel) in (&positions, &velocities).iter() { }

// Sailor (느림): 일부 엔티티만 가진 경우
for (pos, opt_vel) in (&positions, Optional(&velocities)).iter() { }
```

**팁**: 자주 함께 사용되는 컴포넌트는 항상 함께 추가하세요.

### 6. 벤치마크 결과

**Bunny Demo** (10,000 엔티티):
- 반복: ~0.02ms
- 병렬 반복: ~0.008ms (2.5배 빠름, 4 코어)
- 추적 오버헤드: ~5%

**실전 권장사항**:
1. 병렬화: 10,000+ 엔티티부터 효과적
2. 추적: 필요한 것만 활성화
3. 스토리지: 대부분 SparseSet이 최적
4. 버킷: 기본값(32) 유지

---

## Cargo Features

### 기본 기능 (default)
```toml
[dependencies]
shipyard = "0.9"

# 다음과 동일:
# features = ["parallel", "proc", "std"]
```

### 개별 기능

#### parallel
- Rayon 기반 병렬 반복
- 워크로드 병렬 실행
- 오버헤드: ~200KB

```toml
shipyard = { version = "0.9", default-features = false, features = ["parallel", "std"] }
```

#### proc
- `#[derive(Component)]` 매크로
- `#[derive(Unique)]` 매크로
- 편의성 크게 향상

```toml
shipyard = { version = "0.9", default-features = false, features = ["proc", "std"] }
```

#### std
- 표준 라이브러리 사용
- `World::new()` 등 편의 메서드
- 비활성화 시 `no_std` 환경 지원

```toml
# no_std
shipyard = { version = "0.9", default-features = false, features = ["proc"] }
```

#### serde1
- Serde 직렬화/역직렬화
- `EntityId`, 스토리지 저장/로드

```toml
shipyard = { version = "0.9", features = ["serde1"] }
```

#### thread_local
- `!Send`, `!Sync` 컴포넌트 지원
- `NonSend`, `NonSync` 래퍼

```toml
shipyard = { version = "0.9", features = ["thread_local"] }
```

#### tracing
- 워크로드 실행 추적
- 시스템 성능 프로파일링

```toml
shipyard = { version = "0.9", features = ["tracing"] }
```

#### extended_tuple
- 튜플 크기 10 → 32로 확장
- 빌드 시간 4배 증가
- 많은 컴포넌트 동시 접근 시 필요

```toml
shipyard = { version = "0.9", features = ["extended_tuple"] }
```

---

## 추가 리소스

### 공식 문서
- [User Guide](https://leudz.github.io/shipyard/guide/master)
- [API Documentation](https://docs.rs/shipyard)
- [GitHub Repository](https://github.com/leudz/shipyard)

### 커뮤니티
- [Zulip Chat](https://shipyard.zulipchat.com)
- [GitHub Issues](https://github.com/leudz/shipyard/issues)

### 예제
- [Bunny Demo](https://leudz.github.io/shipyard/bunny_demo) - 성능 벤치마크
- [Square Eater](https://leudz.github.io/shipyard/square_eater) - 간단한 게임
- [Visualizer](https://leudz.github.io/shipyard/visualizer) - ECS 시각화

---

## 라이선스

Shipyard는 듀얼 라이선스:
- MIT License
- Apache License 2.0

사용자가 선택 가능합니다.

---

## 기여

기여를 환영합니다! [CONTRIBUTING.md](CONTRIBUTING.md) 참조.

---

**문서 버전**: Shipyard 0.9.0 기준
**작성일**: 2025-11-12
**분석 대상**: master 브랜치 (최신 커밋: 51fcd8e)
