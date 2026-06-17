# Plan: CspIndex 오케스트레이터 배선 및 인덱스 영속화·캐싱 모델

> Track: cspindex-orchestrator-20260617
> Spec: [spec.md](./spec.md)

## Overview

- **Source**: /please:plan
- **Track**: cspindex-orchestrator-20260617
- **Issue**: #18
- **Created**: 2026-06-17
- **Approach**: 4-phase stacked 구현 — (A) 오케스트레이터 in-memory 배선 → (B) 명시 경로 영속화
  roundtrip → (C) 글로벌 `~/.csp/` content-hash 자동 캐시 + ADR → (D) `clear index` 실동작 + 문서 정합
- **Execution**: code
- **Planned At**: d1ff3f6

## Purpose

포팅됐지만 배선되지 않은 인덱싱 유닛을 동작하는 `CspIndex`로 묶고, 디스크 영속화와 글로벌
`~/.csp/` content-hash 자동 캐시를 구현해 `csp index`/`search`/`find-related`와 `clear index`가
엔드투엔드로 동작하게 한다. 저장/캐싱 모델 결정은 ADR로 남긴다.

## Context

`src/indexing/` 유닛(`create`/`files`/`file-walker`/`dense`/`sparse`)은 개별 포팅됐고
`src/search.ts`에 동기 하이브리드 랭킹 파이프라인 `search(query, model, semanticIndex, bm25Index,
chunks, opts)`가 있다. 그러나 `CspIndex`(`src/indexing/index.ts`)는 stub이며
`src/mcp/server.ts`의 `IndexCache`가 이미 `CspIndex.fromPath/fromGit`와 `index.search`(동기 호출)에
의존한다. `cli.ts`는 `index.save(out)`/`CspIndex.loadFromDisk()`를 호출하지만 둘 다 미구현이다.
`~/.csp/` 홈은 `stats.ts`가 `savings.jsonl`로 이미 사용 중이다. `clear index`는 현재 no-op 안내
(`cli.ts` `_runClear`)다.

### 탐색에서 검증된 사실 (구현 전 반영 필수)

1. **create.ts API 불일치 (블로커)**: `create.ts:75-76`이 `new Bm25Index()` + `bm25Index.index(...)`를
   호출하나 `sparse.ts`의 `Bm25Index`는 `private` 생성자 + `static build(documents)`만 노출 →
   현재 배선 시 컴파일/런타임 실패. T002에서 `Bm25Index.build(...)`로 교정.
2. **search 시그니처는 동기**: `search.ts`의 `search()`는 동기, `mcp/server.ts:370`이 `await` 없이
   호출 → `CspIndex.search/findRelated`는 동기 `SearchResult[]` 유지. async로 바꾸면 mcp가 깨진다.
3. **search.ts 타입 중복**: `search.ts`가 `Chunk`/`SearchResult`/`tokenize`를 로컬 정의
   (`TODO(integration)`). 배선 전 `../types.ts`/`../tokens.ts`로 통합(T001).
4. **직렬화 부품**: `Bm25Index.save/load`(dir → `bm25.json`, `version:1`),
   `SelectableBasicBackend.save/load`(dir → `vectors.bin` + `args.json`, **버전 필드 없음**) 존재.
   chunks + top-level `manifest.json`(스키마 버전·content-hash·소스 동일성)을 추가해 NFR-001 충족.
5. **MCP IndexCache**: in-memory LRU(소스 경로 키 + file-watcher). 디스크 캐시와 한 경로로 수렴
   (T012)해 상충 뷰 방지.

### STOP Conditions (플랜 전역)

- 어느 태스크든 `src/types.ts`의 `Chunk` 형상이 `search.ts` 로컬 `Chunk`와 의미적으로 달라 통합이
  비호환이면, 즉흥 변환 대신 멈추고 보고한다.
- 직렬화 roundtrip이 dense 백엔드의 부동소수 비결정성으로 의미적 동등성(NFR-002)을 깨면 멈추고
  보고한다(허용 오차 정책 필요).

## Architecture Decision

**저장/캐싱 모델: 글로벌 `~/.csp/` content-hash 자동 캐시 (+ `-o`/`--index` 명시 경로 존중).**
`product.md`의 upstream 추종 원칙, 기존 `~/.csp/savings.jsonl` 홈 일관성, 로컬 repo가 없는
`fromGit` 대응을 근거로 채택. repo-local `.csp/`(CLAUDE.md 기존 기재) 대비 divergence는 ADR-0002로
기록. 결정 세부는 T013 ADR에서 upstream `cache.py` 실제 소스를 읽어 근거화한다.

**레이어 분리**:
- `CspIndex`(오케스트레이터): `{model, semanticIndex, bm25Index, chunks}` 보유, `fromPath/fromGit`
  (빌드), `search/findRelated`(동기 랭킹), `save/loadFromDisk`(명시 경로 roundtrip).
- `src/indexing/cache.ts`(신규): `resolveCacheDir(source, ref, content)` → `~/.csp/index/<key>`,
  `computeContentHash(files)`, `loadOrBuildIndex(source, opts)`(디스크 캐시 조회·검증·재사용/빌드).
  캐시 키 = 소스 동일성(절대경로 / git URL) + content-hash. 디렉터리 0700 생성.
- CLI: 명시 `-o`/`--index` → `save`/`loadFromDisk` 직접. 미지정 → `loadOrBuildIndex` 자동 캐시.

## Architecture Diagram

```
cli/mcp ─┬─ 명시 -o/--index ──→ CspIndex.save / loadFromDisk ──→ <명시 dir>
         └─ 미지정 ──→ cache.loadOrBuildIndex ──→ ~/.csp/index/<sourceId+contentHash>/
                                                   ├─ manifest.json (schemaVer, hash, sourceId)
                                                   ├─ chunks.json
                                                   ├─ bm25.json   (Bm25Index.save)
                                                   └─ vectors.bin + args.json (dense.save)
CspIndex.fromPath/fromGit → createIndexFromPath → {bm25Index, semanticIndex, chunks} + loadModel
CspIndex.search/findRelated → search.ts search(query, model, semanticIndex, bm25Index, chunks)
```

## Tasks

### Phase A — 오케스트레이터 in-memory 배선 (P1)

- [ ] T001 search.ts 타입/토크나이저를 `../types.ts`·`../tokens.ts`로 통합 (file: src/search.ts)
  STOP: `../types.ts`의 `SearchResult`는 `toDict()`를 요구하나 `search.ts`의 반환은 `{chunk,score}`
  리터럴이고 `utils.ts:62`가 `r.toDict()`를 호출한다. 통합 시 toDict 처리 방식을 먼저 결정한다
  (search 반환 객체에 toDict 부여 vs types.ts에서 toDict 제거하고 출력 경계에서 포맷). `Chunk`/
  `SearchResult` 형상이 비호환이면 즉흥 변환 대신 멈추고 보고
- [ ] T002 [P] create.ts 선존 컴파일 에러 3건 일괄 교정 — `new Bm25Index()`→`Bm25Index.build(...)`,
  `new SelectableBasicBackend(embeddings, model.dim)`의 잘못된 2번째 인자(ctor는 `(vectors, BasicArgs)`),
  `ContentType.Code`→`ContentType.CODE` (file: src/indexing/create.ts)
  STOP: `Bm25Index.build`/`SelectableBasicBackend` ctor 시그니처가 예상과 다르면 멈추고 보고
- [ ] T003 CspIndex.fromPath 구현 — loadModel + createIndexFromPath, `{model, semanticIndex, bm25Index, chunks}` 보유; 동시에 `loadFromDisk`/`save`를 throwing stub으로 선언해 Phase A 브랜치가 cli.ts:415 참조로 typecheck 깨지지 않게 함 (file: src/indexing/index.ts) (depends on T001, T002)
- [ ] T004 CspIndex.search/findRelated를 search.ts 파이프라인에 동기 배선 (file: src/indexing/index.ts) (depends on T003)
- [ ] T005 CspIndex.fromGit 구현 — 원격 체크아웃 후 fromPath 파이프라인 재사용. 임시 체크아웃 디렉터리는 0700으로 생성하고 인덱싱 완료/오류 시 정리한다 (file: src/indexing/index.ts) (depends on T003)
  STOP: 체크아웃 위치가 `.cspignore` 스캔 범위를 벗어나 무시 규칙이 누락되면 멈추고 보고

### Phase B — 명시 경로 영속화 roundtrip (P1)

- [ ] T006 CspIndex.save(dir) 구현 — manifest.json(schemaVersion; contentHash; sourceId; content=ContentType[]; modelId=모델 식별자) + chunks.json + Bm25Index.save + SelectableBasicBackend.save. dense 백엔드는 버전 필드가 없으므로 modelId를 dense args.json에도 기록해 manifest 단일 의존을 줄인다 (file: src/indexing/index.ts) (depends on T003)
  STOP: dense/bm25 save가 같은 dir에 파일명 충돌을 일으키면 멈추고 보고
  STOP: dense `save`가 정규화된 벡터를 쓰고 `load`가 재정규화하여 float drift로 NFR-002(roundtrip 동등성)가 깨지면, 즉흥 처리 말고 멈추고 보고(미정규화 저장 또는 load 시 skipNormalize로 해소)
- [ ] T007 CspIndex.loadFromDisk(dir) 구현 — manifest 검증(스키마 버전·modelId 불일치 시 오류), chunks/bm25/dense 복원 + 모델 재로드 (file: src/indexing/index.ts) (depends on T006)
- [ ] T008 cli index `-o`·search/find-related `--index`를 save/loadFromDisk에 배선(명시 경로 존중) (file: src/cli.ts) (depends on T007)

### Phase C — 글로벌 content-hash 자동 캐시 + ADR (P2)

- [ ] T009 cache 모듈 신규 — resolveCacheDir(`~/.csp/index/<key>`), computeContentHash(정렬 매니페스트: 상대경로+내용), 캐시 키에 소스 동일성 포함. `~/.csp/`(이미 stats.ts가 mode 없이 생성)·`~/.csp/index/`·leaf까지 0700 보장(`mkdir {recursive,mode:0o700}` + 기존 디렉터리는 chmod) (file: src/indexing/cache.ts) (depends on T006)
  STOP: content-hash 입력이 `fromGit`에서 비결정적(체크아웃 메타데이터 포함)이면 멈추고 보고 — 폴백으로 git commit SHA를 소스 키에 사용
- [ ] T010 loadOrBuildIndex 자동 캐시 — 디스크 캐시 조회·content-hash 검증·재사용/빌드+저장·무효화 (file: src/indexing/cache.ts) (depends on T009, T007)
- [ ] T011 cli **search/find-related**(명시 경로 없음)를 loadOrBuildIndex 자동 캐시에 배선. `csp index`는 명시 `-o`를 계속 요구(명시 영속화 전용) — 자동 캐시 대상 아님 (file: src/cli.ts) (depends on T010, T008)
- [ ] T012 mcp IndexCache를 디스크 캐시(loadOrBuildIndex)와 정합화 — 상충 뷰 방지. **T011과 같은 Phase C PR로 함께 머지**(T011만 단독 머지 시 CLI↔MCP 캐시 분기) (file: src/mcp/server.ts) (depends on T010)
  STOP: file-watcher 무효화와 디스크 content-hash 무효화가 이중 재빌드를 일으키면 멈추고 보고. 무효화 소유권(인메모리 evict가 디스크 엔트리도 지우는지)을 먼저 정한다
- [ ] T013 저장/캐싱 모델 ADR 작성 — upstream `cache.py` 실제 소스 근거화, divergence 기록 (file: .please/docs/decisions/0002-index-storage-cache-model.md) (depends on T009)

### Phase D — clear index 실동작 + 문서 정합 (P2)

- [ ] T014 `_runClear` index/all을 배선 — 삭제 대상은 **오직 `~/.csp/index/`**(`~/.csp/` 루트 rmtree 금지). `clear all`은 `~/.csp/index/` 삭제 **후** `clearSavings()`를 독립 호출. 제거 항목 수/용량 보고, savings 보존 (file: src/cli.ts) (depends on T009)
  STOP: 삭제 경로가 `~/.csp/` 루트 또는 `~/.csp/savings.jsonl`을 포함하면 멈추고 보고(AC-015 위반)
- [ ] T015 README.md/README.ko.md clear·index·savings 갱신 + CLAUDE.md(.csp 노트, `.load`→`loadFromDisk`, file-walker 비의존 확인) (files: README.md, README.ko.md, CLAUDE.md) (depends on T011, T014)

## Dependencies

```
T001 ─┐
T002 ─┴→ T003 → T004
              └→ T005
       T003 → T006 → T007 → T008
                     T006 → T009 → T010 → T011 (← T008)
                                   T010 → T012
                                   T009 → T013
                                   T009 → T014
                            T011, T014 → T015
```

Phase 경계(A→B→C→D)는 stacked PR 분기점. T002는 [P], 나머지는 의존 체인.

## Key Files

- `src/indexing/index.ts` — CspIndex 오케스트레이터 (fromPath/fromGit/search/findRelated/save/loadFromDisk)
- `src/indexing/create.ts` — createIndexFromPath (Bm25Index.build 교정 대상)
- `src/indexing/cache.ts` — **신규**: 캐시 위치 해석·content-hash·loadOrBuildIndex
- `src/indexing/sparse.ts` / `src/indexing/dense.ts` — Bm25Index/SelectableBasicBackend save/load 재사용
- `src/search.ts` — 동기 하이브리드 랭킹 파이프라인 (타입 통합 대상)
- `src/cli.ts` — index/search/find-related/clear 배선
- `src/mcp/server.ts` — IndexCache 정합화
- `src/stats.ts` — `~/.csp/` 홈 패턴 참조 (savings 보존 경계)
- `.please/docs/decisions/0002-index-storage-cache-model.md` — **신규** ADR

## Verification

- 각 태스크 RED-GREEN-REFACTOR, `bun test` 통과.
- `bun run typecheck`: **baseline가 이미 red**(tsconfig `.ts` import 확장자 TS5097, `types.test.ts`의
  부재 헬퍼 등 선존 에러). 따라서 게이트는 "전체 green"이 아니라 **변경 모듈에 새 타입 에러 없음**
  으로 한다. 특히 T001/T002는 `create.ts`/`search.ts`의 선존 에러를 줄여야 한다(증가 금지).
- 엔드투엔드: 샘플 repo에서 `csp index -o /tmp/idx` → `csp search --index /tmp/idx` 비어있지 않은 결과(SC-001a).
- 자동 캐시: 동일 repo 2회 검색 시 2회차가 인덱싱 단계 skip, **2회차 전체 소요 ≤ 1회차의 10%**(SC-002), 파일 변경 후 갱신 반영(SC-003).
- `csp clear index` 후 `~/.csp/index` 비워지고 `~/.csp/savings.jsonl` 보존(SC-004).
- README(영/한) clear·index·savings 설명이 실제 동작과 일치(SC-005).

## Test Scenarios

### T001
- Happy: 기존 search.test.ts가 통합 타입으로도 동일 통과 → `bun test src/search.test.ts` green.
- Edge: `Chunk.language`가 `null`/`undefined`인 청크가 랭킹에서 동일 처리.

### T002
- Happy: createIndexFromPath가 `Bm25Index.build`로 비어있지 않은 bm25 인덱스 생성 → create.test.ts green.
- Error: 지원 파일 0개 → 기존 "No supported files" 오류 유지.

### T003
- Happy: 샘플 디렉터리 fromPath → chunks/semanticIndex/bm25Index/model 채워진 CspIndex 반환(AC-001).
- Error: 지원 파일 없는 경로 → 명확한 오류로 실패(AC-005).

### T004
- Happy: 알려진 심볼 쿼리 search → 기대 청크 상위, 점수 내림차순(AC-002). findRelated → 유사 청크(AC-003).
- Edge: topK가 후보보다 크면 가능한 결과 전부 반환.

### T005
- Happy: 작은 git URL fromGit → 채워진 인덱스(AC-004).
- Error: 잘못된 URL → 명확한 오류.

### T006
- Happy: save(dir) → manifest.json/chunks.json/bm25.json/vectors.bin/args.json 생성, manifest에 schemaVersion·contentHash·sourceId 포함(AC-006).
- Integration: dense+bm25 save가 같은 dir에 공존.

### T007
- Happy: save→loadFromDisk roundtrip 후 동일 쿼리가 저장 직전과 동일 상위 결과(AC-007, NFR-002).
- Error: 스키마 버전 불일치/손상 manifest → 빈 결과 아닌 명확한 오류(AC-009, NFR-001).

### T008
- Happy: `csp index -o <out>` 후 `csp search --index <out>` 동작, 명시 경로 사용(AC-008, SC-001a).
- Error: 존재하지 않는 `--index` 경로 → 명확한 오류.

### T009
- Happy: resolveCacheDir이 `~/.csp/index/<key>` 반환, 동일 (소스, 내용)에 결정적 키, 디렉터리 0700(NFR-003).
- Edge: 동일 content-hash·다른 소스(경로/URL) → 다른 키(NFR-004).

### T010
- Happy: 미존재 캐시 → 빌드+저장; 존재·해시 일치 → 재인덱싱 없이 재사용(AC-010, AC-011).
- Edge: 콘텐츠 변경 → content-hash 불일치로 무효화 후 재인덱싱(AC-012, SC-003).

### T011
- Happy: `csp search <query>`(명시 인덱스 없음) → 자동 캐시 사용 엔드투엔드(SC-001b). 2회차 인덱싱 skip(SC-002).

### T012
- Happy: mcp `IndexCache.get` 경로가 디스크 캐시와 일치된 인덱스 반환, 이중 빌드 없음.
- Integration: file-watcher 무효화 후 재빌드가 디스크 캐시도 갱신.

### T013
- Test expectation: none -- ADR 문서 작성(산출물). 검증: ADR이 결정·근거(upstream cache.py 인용)·divergence·대안을 담고 `.please/docs/decisions/`에 저장, index.md 갱신.

### T014
- Happy: 캐시 채운 뒤 `csp clear index` → `~/.csp/index` 비워지고 제거 항목/용량 보고(AC-013, SC-004).
- Edge: 비울 캐시 없음 → 오류 없이 "비울 캐시 없음" 안내(AC-014).
- Error/경계: savings.jsonl 미삭제 보장(AC-015).

### T015
- Test expectation: none -- 문서 갱신. 검증: README 영/한 clear·index·savings가 실제 CLI 동작과 일치(SC-005), CLAUDE.md `.load`→`loadFromDisk` 정합·`.csp` 노트 갱신, 영/한 동기화.

## Progress

- [x] (2026-06-18 09:00 KST) T001 search.ts 타입/토크나이저를 `../types.ts`·`../tokens.ts`로 통합

## Decision Log

- 저장 모델: 글로벌 `~/.csp/` content-hash 자동 캐시 + 명시 경로 존중 (사용자 확정, ADR-0002 예정).
- search/findRelated: 동기 시그니처 유지 (search.ts·mcp 동기 호출 정합).
- 범위: 이슈 4개 그룹 전체, A→D phase 분할(stacked PR). 자동 eviction은 후속 트랙.
- 자동 캐시 대상: `search`/`find-related`만. `csp index`는 명시 `-o` 유지(명시 영속화 전용).
- T011(cli 자동캐시)과 T012(mcp 정합)는 같은 Phase C PR로 함께 머지(분기 뷰 방지).
- T001 `SearchResult.toDict` 처리 방식(search 반환에 부여 vs types.ts에서 제거)은 T001 착수 시 확정.
  → 확정(2026-06-18): types.ts의 `SearchResult{chunk,score,toDict}` 형상 유지. search.ts에 작은
  헬퍼(`makeResult`/`chunkToDict`)를 두어 `search`/`_searchSemantic`/`_searchBm25`의 모든 생성 지점이
  `toDict`를 부여. `toDict` 형상은 `utils.formatResults`가 소비하는 `{chunk: <snake_cased+location>, score}`
  (utils.test.ts·mcp/server.test.ts 계약과 일치). 근거: 다운스트림(utils.ts:62 `r.toDict()`)이 이미 의존하고,
  types.ts에 공유 헬퍼가 없어 모듈 로컬 헬퍼가 최소 변경.
  Date/Author: 2026-06-18 / implement-executor
- 플랜 리뷰(coherence/feasibility/completeness/scope-guardian/security/adversarial)에서
  create.ts 선존 컴파일 에러 3건·타입통합 toDict cascade·Phase A typecheck stub·캐시 권한 하드닝을
  반영(2026-06-17).

## Surprises & Discoveries

- create.ts의 Bm25Index API 불일치(컴파일 블로커) — Phase 탐색에서 발견, T002로 선반영.
- search.ts 타입 중복(`TODO(integration)`) — 배선 전 통합 필요(T001).
- mcp IndexCache가 이미 오케스트레이터에 의존 — 디스크 캐시 정합화 필요(T012).
- T001: `../tokens.ts`의 `tokenize`는 search.ts 로컬 tokenize와 동작 동등(동일 TOKEN_RE/CAMEL_RE/
  splitIdentifier 로직; tokens.ts는 순수 소문자 토큰 fast-path만 추가하나 출력 동일). 교체 후
  `bun test src/search.test.ts` 24 pass(기존 20 + toDict 4), 전체 스위트 fail 집합 불변(316→320 pass,
  5 fail/3 errors 동일) — 토크나이저 회귀 없음 확인.
