---
product_spec_domain: indexing
---

# CspIndex 오케스트레이터 배선 및 인덱스 영속화·캐싱 모델

> Track: cspindex-orchestrator-20260617
> Origin issue: pleaseai/code-search#18

## Overview

오늘 csp는 인덱싱 유닛(`create`, `files`, `file-walker`, `dense`, `sparse`)을 각각 포팅했지만,
이를 묶어 실제로 검색 가능한 인덱스를 만드는 **오케스트레이터(`CspIndex`)가 비어 있다.**
`CspIndex.fromPath`/`fromGit`은 `not yet implemented` 예외를 던지고, `save()`/`loadFromDisk()`는
존재하지 않으며, `search`/`findRelated`는 빈 배열을 반환한다. 그 결과 `csp search`/`csp index`가
엔드투엔드로 동작하지 않고, 어떤 형태의 인덱스 캐시도 없다.

이 트랙은 (1) 포팅된 유닛을 동작하는 `CspIndex` 파이프라인으로 배선하고, (2) 인덱스를 디스크에
저장/복원하는 영속화 라운드트립을 구현하며, (3) **인덱스 저장/캐싱 모델을 글로벌 `~/.csp/`
content-hash 캐시로 확정(ADR 기록)**하고, (4) 그 모델에 따라 자동 인덱싱·캐시 무효화·`csp clear
index` 실동작·문서를 정합화한다.

**확정된 결정 (ADR 대상):** 인덱스는 **글로벌 `~/.csp/` 홈 아래 content-hash 서브디렉터리**에
자동 캐시한다. 이는 upstream semble의 글로벌 캐시 방향(#162/#177/#178/#182)과 일치하고, 기존
`~/.csp/savings.jsonl` 홈 규약과 일관되며, 로컬 repo가 없는 `fromGit` 시나리오를 자연스럽게
지원한다. 단, 사용자가 `-o`/`--index`로 **명시한 경로는 항상 존중**한다(수동 라운드트립).
upstream 추종 원칙(`product.md`)에 부합하며, CLAUDE.md의 repo-local `.csp/` 무시 항목은
vestigial이 되므로 ADR에서 갱신 사유를 기록한다.

## User Scenarios & Testing

### User Story 1 — 라이브러리 사용자가 경로에서 검색 가능한 인덱스를 만든다 (Priority: P1)

에이전트 하니스/스크립트를 작성하는 개발자가 `CspIndex.fromPath(dir)`를 호출하면, 포팅된
인덱싱 유닛이 배선되어 실제로 검색 가능한 인덱스를 돌려받는다. 이어 `.search(query)`가 dense+BM25
하이브리드 랭킹 결과를 반환한다.

**Why this priority**: 오케스트레이터 배선이 없으면 나머지 모든 시나리오(영속화·캐싱·CLI)가
성립하지 않는 기반 작업이다. 라이브러리 표면(`CspIndex.fromPath/.search/.findRelated`)은 README의
공개 계약이다.

**Independent Test**: 샘플 디렉터리로 `CspIndex.fromPath`를 호출해 비어 있지 않은 인덱스를 얻고,
알려진 심볼/NL 쿼리로 `.search`가 기대 청크를 상위에 반환하는지 확인한다(캐시·CLI 없이 단독 검증).

**Acceptance Criteria** (EARS):

1. **AC-001** — `CspIndex.fromPath(path)`가 호출되면, 시스템은 포팅된 인덱싱 유닛을 배선해
   채워진(인덱스된) `CspIndex` 인스턴스를 반환해야 한다.
2. **AC-002** — 채워진 인덱스에 대해 `.search(query)`가 호출되면, 시스템은 dense+BM25 하이브리드
   랭킹 파이프라인을 거친 `SearchResult` 목록을 점수 내림차순으로 반환해야 한다.
3. **AC-003** — 채워진 인덱스에 대해 `.findRelated(chunk)`가 호출되면, 시스템은 해당 청크와
   의미적으로 유사한 `SearchResult` 목록을 반환해야 한다.
4. **AC-004** — `CspIndex.fromGit(url)`가 호출되면, 시스템은 원격 저장소를 체크아웃하여 동일한
   인덱싱 파이프라인으로 채워진 인덱스를 반환해야 한다.
5. **AC-005** — 지원하는 파일(파일 워커가 매칭하는 확장자 집합)이 하나도 없는 경로가 주어지면,
   시스템은 빈 인덱스를 조용히 반환하는 대신 명확한 오류 메시지로 실패해야 한다.

### User Story 2 — 인덱스를 디스크에 저장하고 다시 불러온다 (Priority: P1)

CLI/CI 사용자가 `csp index <path> -o <out>`로 인덱스를 한 번 만들어 저장하고, 이후
`csp search <query> --index <out>`로 재빌드 없이 같은 인덱스를 불러 검색한다.

**Why this priority**: `csp index -o`와 `--index` 플래그는 README에 문서화된 공개 계약이며,
영속화 라운드트립이 없으면 CLI가 동작하지 않는다. 자동 캐싱도 이 직렬화 형식 위에 세워진다.

**Independent Test**: 인덱스를 만들어 `save()`로 디스크에 쓰고, 새 프로세스에서 `loadFromDisk()`로
복원한 뒤 동일 쿼리가 저장 전과 동일한 상위 결과를 내는지 비교한다.

**Acceptance Criteria** (EARS):

1. **AC-006** — `CspIndex.save(path)`가 호출되면, 시스템은 인덱스(청크 + dense + BM25 상태)를
   해당 경로에 직렬화해 기록해야 한다.
2. **AC-007** — `CspIndex.loadFromDisk(path)`가 호출되면, 시스템은 저장된 인덱스를 복원해 저장
   직전과 의미적으로 동일한 검색 결과를 내는 인스턴스를 반환해야 한다.
3. **AC-008** — 사용자가 `-o`/`--index`로 명시 경로를 제공하면, 시스템은 글로벌 캐시 위치 대신
   그 경로를 사용해야 한다.
4. **AC-009** — 손상되었거나 호환되지 않는(스키마 버전 불일치) 인덱스 파일을 불러오면, 시스템은
   조용히 빈 결과를 반환해서는 안 되며 명확한 오류로 실패해야 한다.

### User Story 3 — 인덱스가 자동으로 캐시되고 콘텐츠 변경 시 무효화된다 (Priority: P2)

에이전트/개발자가 `csp search <query>`를 명시 인덱스 없이 실행하면, 시스템이 글로벌 `~/.csp/`
캐시에서 해당 콘텐츠의 인덱스를 찾아 재사용하고, 없거나 콘텐츠가 바뀌었으면 다시 인덱싱해 캐시한다.

**Why this priority**: upstream의 핵심 UX(자동 인덱스 재사용)를 따라가는 부분으로, 반복 검색의
지연을 없앤다. P1(배선·영속화)이 선행되어야 성립한다.

**Independent Test**: 같은 디렉터리에서 검색을 두 번 실행해 두 번째가 재빌드 없이 캐시를 재사용함을
확인하고, 파일을 변경한 뒤 검색하면 인덱스가 갱신됨을 확인한다.

**Acceptance Criteria** (EARS):

1. **AC-010** — 명시 인덱스 없이 검색/인덱싱이 요청되면, 시스템은 인덱스를 글로벌 `~/.csp/` 홈
   아래 콘텐츠 해시 기반 서브디렉터리에 캐시해야 한다.
2. **AC-011** — 캐시된 인덱스의 콘텐츠 해시가 현재 대상 콘텐츠와 일치하면, 시스템은 재인덱싱 없이
   캐시된 인덱스를 재사용해야 한다.
3. **AC-012** — 대상 콘텐츠의 content-hash가 캐시된 인덱스의 content-hash와 달라지면(파일
   추가/삭제/내용 변경 포함), 시스템은 캐시를 무효화하고 다시 인덱싱해야 한다.

### User Story 4 — 사용자가 인덱스 캐시를 비운다 (Priority: P2)

사용자가 `csp clear index`를 실행하면 글로벌 인덱스 캐시가 실제로 삭제된다(현재는 no-op 안내만).
`csp clear savings`는 기존대로 동작한다.

**Why this priority**: 디스크 점유 회수와 강제 재인덱싱 수단. 캐시 모델(US3)이 확정되어야 무엇을
지울지 정의된다.

**Independent Test**: 캐시를 채운 뒤 `csp clear index`를 실행하면 캐시 디렉터리가 비워지고, 이어진
검색이 재인덱싱을 트리거하는지 확인한다.

**Acceptance Criteria** (EARS):

1. **AC-013** — `csp clear index`가 실행되면, 시스템은 글로벌 `~/.csp/` 인덱스 캐시를 삭제하고
   삭제 결과(제거된 항목/용량)를 보고해야 한다.
2. **AC-014** — 비울 인덱스 캐시가 없는 상태에서 `csp clear index`가 실행되면, 시스템은 오류 없이
   "비울 캐시 없음"을 안내해야 한다.
3. **AC-015** — `csp clear index`는 `~/.csp/savings.jsonl`(savings 데이터)을 삭제해서는 안 된다.

## Requirements

### Functional Requirements

- **FR-001**: 시스템은 `CspIndex.fromPath`/`fromGit`에서 포팅된 인덱싱 유닛(create/files/
  file-walker/dense/sparse)을 배선해 검색 가능한 인덱스를 생성해야 한다.
- **FR-002**: 시스템은 `CspIndex.search`/`findRelated`가 dense+BM25 하이브리드 랭킹(RRF, adaptive
  alpha, 경로 패널티 등 semble 규약)을 거친 결과를 반환하도록 해야 한다. 두 API는 `src/search.ts`
  파이프라인과 `src/mcp/server.ts`(동기 호출, no await)에 맞춰 **동기** `SearchResult[]`를 반환한다
  (cli.ts의 `await`는 동기 반환에도 무해).
- **FR-003**: 시스템은 `CspIndex.save(path)`와 `CspIndex.loadFromDisk(path)`로 인덱스를
  디스크에 직렬화/복원하는 라운드트립을 제공해야 한다.
- **FR-004**: 시스템은 명시 경로(`-o`/`--index`)가 주어지면 그 경로를, 아니면 글로벌 `~/.csp/`
  content-hash 캐시 위치를 사용해야 한다.
- **FR-005**: 시스템은 명시 인덱스 없이 호출될 때 콘텐츠 해시 기반으로 캐시를 조회·재사용하고,
  콘텐츠 변경 시 무효화 후 재인덱싱해야 한다.
- **FR-006**: 시스템은 `csp clear index`가 글로벌 인덱스 캐시를 실제로 삭제하도록 해야 하며,
  savings 데이터에는 영향을 주지 않아야 한다.
- **FR-007 (P2)**: 시스템은 저장/캐싱 모델 결정을 `.please/docs/decisions/` 하위 ADR로 기록하고,
  CLAUDE.md의 repo-local `.csp/` 관련 기재를 결정에 맞게 갱신해야 한다. CLAUDE.md 무시 항목을
  갱신하기 전, 파일 워커(`src/indexing/file-walker.ts`)가 repo-local `.csp/`에 의존하지 않음을
  확인해야 한다.
- **FR-008 (P2)**: 시스템은 `README.md`/`README.ko.md`의 `clear`·`index`/savings 섹션을 실제 동작과
  일치하도록 갱신해야 한다(영/한 동기화). 이때 라이브러리 API 명칭 불일치(문서의 `CspIndex.load()`
  표기 → 실제 코드의 `loadFromDisk()`)를 CLAUDE.md/README에서 일관되게 정합화한다.

### Non-functional Requirements

- **NFR-001**: 저장 인덱스 형식은 스키마 버전을 포함해, 비호환 인덱스를 조용히 오작동시키지 않고
  감지·실패할 수 있어야 한다.
- **NFR-002**: 디스크 직렬화/복원은 인메모리 재인덱싱 대비 검색 결과의 의미적 동등성을 보존해야
  한다(라운드트립 무손실).
- **NFR-003**: 글로벌 캐시 디렉터리는 소유자 전용 권한(0700)으로 생성해, 공유 시스템(멀티유저
  개발기·네트워크 홈)에서 타 사용자가 캐시된 소스 콘텐츠를 읽을 수 없어야 한다.
- **NFR-004**: 캐시 키는 content-hash와 함께 소스 동일성(`fromPath`의 절대 경로 / `fromGit`의
  저장소 URL)을 포함해, 우연히 동일한 file-set 해시를 갖는 서로 다른 소스가 캐시 엔트리를 공유해서는
  안 된다.

## Success Criteria

- **SC-001a** (P1): `csp index <path> -o <out> → csp search <query> --index <out>`와 `csp find-related`가
  실제 저장소에서 엔드투엔드로 동작하며 비어 있지 않은 관련 결과를 반환한다(명시 인덱스 경로 기준).
- **SC-001b** (P2): `csp search <query>`를 명시 인덱스 없이 실행하면 글로벌 캐시를 재사용/생성하여
  엔드투엔드로 동작한다.
- **SC-002**: 동일 콘텐츠에 대한 2회차 검색이 재인덱싱 없이 캐시를 재사용하여, 2회차의 전체 소요가
  1회차(인덱싱+검색) 대비 10% 이하로 줄어든다(캐시 히트가 인덱싱 단계를 건너뜀).
- **SC-003**: 콘텐츠를 변경한 뒤의 검색이 갱신된 결과를 반영한다(stale 결과 없음).
- **SC-004**: `csp clear index` 실행 후 인덱스 캐시가 비워지고 savings 데이터는 보존된다.
- **SC-005**: README(영/한)의 clear·index·savings 설명이 실제 CLI 동작과 일치한다.

## Out of Scope

- 멀티-repo/모노레포를 단일 인덱스로 묶는 인덱싱(별도 비목표).
- MCP 세션 수명을 넘는 영속 서버 모드.
- 캐시 용량 상한/자동 eviction 정책(이번 트랙은 명시적 `clear index`까지; 자동 GC는 후속 고려).
- 캐시 위치를 env/플래그로 임의 override하는 기능(이번 트랙은 글로벌 기본 + `-o`/`--index` 명시
  경로까지; 범용 위치 override는 후속 고려).
- 사용자 정의 임베딩 모델/학습.

## Assumptions

- `~/.csp/`는 이미 savings(`~/.csp/savings.jsonl`)에서 쓰는 csp 홈으로, 인덱스 캐시도 같은 홈
  아래 둔다.
- content-hash는 대상 콘텐츠로부터 결정적으로 산출 가능하다고 가정한다. **최소 입력 제약**: 파일
  워커가 매칭하는 모든 파일의 (상대경로, 파일 내용)을 정규 정렬한 매니페스트를 해싱한다. mtime
  단독은 git 체크아웃 후 보존되지 않아 `fromGit`에서 부정확하므로 입력으로 쓰지 않는다. 구체적
  해시 알고리즘·디렉터리 레이아웃은 plan/ADR에서 확정한다.
- upstream semble의 글로벌 캐시 레이아웃을 참조 기준으로 삼되, csp 홈(`~/.csp/`)에 맞게 적응한다.
  ADR 작성 전 upstream 캐시 모듈(`ask src github:MinishLab/semble@main` → `cache.py` 등) 실제
  소스를 읽어 정렬 기준을 근거화하고, 차이가 있으면 divergence로 기록한다(`product.md` 원칙).
- 오케스트레이터(`CspIndex.fromPath/fromGit`)는 이미 존재하는 MCP 인메모리 인덱스 캐시
  (`src/mcp/server.ts`의 `IndexCache`, 소스 경로 키 + `get`/`evict`)와 정합화한다 — 디스크 캐시와
  인메모리 캐시가 상충하는 뷰를 만들지 않도록 한 경로로 수렴시킨다.
- 포팅된 인덱싱 유닛이 모두 동작한다고 가정하지 않는다 — `src/indexing/create.ts`는 현재
  `new Bm25Index()` + `bm25Index.index(...)`를 호출하나 `Bm25Index`는 `private` 생성자 +
  `static build()`만 노출하므로 배선 전 수정이 필요하다(plan에서 처리).
- High effort 트랙으로, stacked PR phase 분할(배선 → 영속화 → ADR/캐싱 → clear/README)을 통해
  점진적으로 구현한다.
- **알려진 리스크(범위 외 완화)**: 글로벌 캐시는 자동 eviction이 이번 범위 밖이라 브랜치/커밋마다
  엔트리가 누적될 수 있다. `csp clear index`가 유일한 회수 수단이며 제거 항목/용량을 보고한다
  (AC-013). 장수명 CI 머신용 LRU/TTL eviction은 후속 트랙 대상이다.
