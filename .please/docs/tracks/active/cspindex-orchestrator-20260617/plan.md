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

- [x] T001 search.ts 타입/토크나이저를 `../types.ts`·`../tokens.ts`로 통합 (file: src/search.ts)
  STOP: `../types.ts`의 `SearchResult`는 `toDict()`를 요구하나 `search.ts`의 반환은 `{chunk,score}`
  리터럴이고 `utils.ts:62`가 `r.toDict()`를 호출한다. 통합 시 toDict 처리 방식을 먼저 결정한다
  (search 반환 객체에 toDict 부여 vs types.ts에서 toDict 제거하고 출력 경계에서 포맷). `Chunk`/
  `SearchResult` 형상이 비호환이면 즉흥 변환 대신 멈추고 보고
- [x] T002 create.ts 선존 컴파일 에러 일괄 교정 + dense/sparse 타입 통합 — (a) `new Bm25Index()`→`Bm25Index.build(...)`, `new SelectableBasicBackend(embeddings, model.dim)` 2번째 인자 제거, `ContentType.Code`→`CODE`; (b) async 정합: `walkFiles`는 `async function*` → `for await`, `chunkSource`는 `async` → `await`, `detectLanguage` 반환 `string|undefined` → `?? null`; (c) `dense.ts`·`sparse.ts`의 로컬 `Chunk` 정의를 `../types.ts` import로 통합(T001과 동일 패턴, 동작 보존) — embedChunks Chunk 타입 불일치 해소 (files: src/indexing/create.ts, src/indexing/dense.ts, src/indexing/sparse.ts)
  STOP: `Bm25Index.build`/`SelectableBasicBackend` ctor 시그니처 또는 dense/sparse Chunk 형상이 types.ts와 비호환이면 멈추고 보고
- [x] T003 CspIndex.fromPath 구현 — loadModel + createIndexFromPath, `{model, semanticIndex, bm25Index, chunks}` 보유; 동시에 `loadFromDisk`/`save`를 throwing stub으로 선언해 Phase A 브랜치가 cli.ts:415 참조로 typecheck 깨지지 않게 함 (file: src/indexing/index.ts) (depends on T001, T002)
- [x] T004 CspIndex.search/findRelated를 search.ts 파이프라인에 동기 배선 + index.test.ts setup을 실제 모듈 API에 정렬(동작 단언 유지, 약화 금지) — 스캐폴드 테스트가 추측한 `new Bm25Index(docs)`→`Bm25Index.build(docs)`, `new SelectableBasicBackend(vecs,4)`→`new SelectableBasicBackend(vecs)`, `makeStubModel('name',4)`→`makeStubModel(4)`(dense.ts에서 export), `ContentType.Code`→`CODE`로 교정 (files: src/indexing/index.ts, src/indexing/index.test.ts, src/indexing/dense.ts) (depends on T003)
- [x] T005 CspIndex.fromGit 구현 — 원격 체크아웃 후 fromPath 파이프라인 재사용. 임시 체크아웃 디렉터리는 0700으로 생성하고 인덱싱 완료/오류 시 정리한다 (file: src/indexing/index.ts) (depends on T003)
  STOP: 체크아웃 위치가 `.cspignore` 스캔 범위를 벗어나 무시 규칙이 누락되면 멈추고 보고
- [x] T0A 선존 테스트-스위트 부채 정리(사용자 승인, 구현 중 발견) — (a) `src/mcp/server.test.ts`의 전역 `mock.module('../indexing/index.ts')` 누수 수정(afterAll 복원 등); (b) types.ts에 canonical **camelCase round-trip 직렬화** 헬퍼 추가(`chunkToDict`/`chunkFromDict`/`chunkLocation`/`searchResultToDict`/`ChunkDictInput`, `types.test.ts`가 정의하는 동작에 정확히 맞춤 — chunkToDict는 camelCase + location, chunkFromDict는 location 제거 후 복원). **이는 search.ts의 `SearchResult.toDict`(snake_case wire 포맷, Decision Log 확정)와 별개의 레이어이므로 search.ts의 toDict는 건드리지 않는다**(재사용 강제 금지 — 두 직렬화는 목적이 다름); (c) 스캐폴드 테스트를 문서화된 소스 계약에 정렬 — `ContentType.Code/Docs/Config`→`CODE/DOCS/CONFIG`, `CallType.Search/FindRelated`→`SEARCH/FIND_RELATED`(CLAUDE.md 계약); (d) `Bm25Index`에 read-only `documents` getter 추가(또는 create.test 정렬); (e) `cli.test.ts` stub-mock 결과에 `toDict` 부여 (files: src/types.ts, src/search.ts, src/indexing/sparse.ts, src/mcp/server.test.ts, src/types.test.ts, src/index.test.ts, src/indexing/create.test.ts, src/cli.test.ts) (depends on T004)
  STOP: ContentType/CallType 케이싱을 소스(CODE/SEARCH)로 정렬하는 것이 README/CLAUDE.md 공개 계약과 충돌하면 멈추고 보고(현재는 CLAUDE.md가 CODE/DOCS/CONFIG 명시 → 소스가 계약)

### Phase B — 명시 경로 영속화 roundtrip (P1)

- [x] T006 CspIndex.save(dir) 구현 — manifest.json(schemaVersion; contentHash; sourceId; content=ContentType[]; modelId=모델 식별자) + chunks.json + Bm25Index.save + SelectableBasicBackend.save. dense 백엔드는 버전 필드가 없으므로 modelId를 dense args.json에도 기록해 manifest 단일 의존을 줄인다 (file: src/indexing/index.ts) (depends on T003)
  STOP: dense/bm25 save가 같은 dir에 파일명 충돌을 일으키면 멈추고 보고
  STOP: dense `save`가 정규화된 벡터를 쓰고 `load`가 재정규화하여 float drift로 NFR-002(roundtrip 동등성)가 깨지면, 즉흥 처리 말고 멈추고 보고(미정규화 저장 또는 load 시 skipNormalize로 해소)
- [x] T007 CspIndex.loadFromDisk(dir) 구현 — manifest 검증(스키마 버전·modelId 불일치 시 오류), chunks/bm25/dense 복원 + 모델 재로드 (file: src/indexing/index.ts) (depends on T006)
- [x] T008 cli index `-o`·search/find-related `--index`를 save/loadFromDisk에 배선(명시 경로 존중) (file: src/cli.ts) (depends on T007)

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
- [x] (2026-06-18 10:30 KST) T002 create.ts 선존 컴파일 에러 3건 교정 (Bm25Index.build / SelectableBasicBackend(embeddings) / ContentType.CODE) — 3개 타깃 에러 제거, 신규 에러 0, 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). create.test.ts green 목표는 **미달**: 테스트가 범위 밖 API에 의존(아래 Surprises 참조)
- [x] (2026-06-18 11:10 KST) T002 (round 2) create.ts 잔존 컴파일 에러 4건 마무리 + dense/sparse Chunk 타입 통합 — async 정합(for await walkFiles / await chunkSource / detectLanguage ?? null), dense.ts·sparse.ts 로컬 `Chunk` 제거 후 `../types.ts`로 통합(re-export 유지). create.ts 소스 4-에러 전부 제거(TS5097 제외 0), 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). 잔존 makeStubModel/DEFAULT_CONTENT 미export 의존 테스트는 범위 밖이라 미수정. commit a328727
- [x] (2026-06-18 12:20 KST) T003 CspIndex.fromPath 배선 + save/loadFromDisk throwing stub — fromPath가 `loadDenseModel(opts.modelPath)` + `createIndexFromPath(path,{model,content,displayRoot:path})`로 `{model,semanticIndex,bm25Index,chunks}` 보유 인스턴스 반환. 생성자를 옵션 객체 `{model,bm25Index,semanticIndex,chunks,modelPath,root,content}`로 확장, `DEFAULT_CONTENT` export, `stats` getter 추가. `save`/`loadFromDisk`는 throwing stub(T006/T007) — cli.ts:288 `index.save` / cli.ts:415 `CspIndex.loadFromDisk` **선존 타입 에러 2건 제거**(stash 비교로 확인). `loadModel`은 `[model,modelPath]` 튜플 재export(mcp `[, modelPath]` 정합), search/findRelated는 동기 stub([]) 유지(T004). 게이트: index.ts 신규 non-TS5097 에러 0, mcp/server.ts·cli.ts 에러 집합 baseline 동일(상기 2건 제거 외), 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). STOP(호출부 시그니처 충돌) 미발동. commit 7aa721a
- [x] (2026-06-18 14:05 KST) T004 CspIndex.search/findRelated 배선 + index.test.ts setup 정렬 — `search()`는 blank query / `topK<=0` / 빈 인덱스 / 빈 selector(필터 무매치) 가드 후 `search.ts`의 `search(query, model, semanticIndex, bm25Index, chunks, topK, {selector?})`에 **동기** 위임(mcp/server.ts:370 await 없이 호출 정합). `findRelated(seed|{chunk,score})`는 시드 content를 재임베딩→`semanticIndex.query(emb, topK+1)`→시드 청크 제외 후 topK 반환. `filterLanguages`/`filterPaths`→`buildSelector`가 후보 인덱스 `Uint32Array` 생성, 무매치 시 길이 0 → `[]`(unfiltered 폴백 없음, 회귀 테스트 충족). `makeStubModel`을 dense.ts에서 export, index.test.ts setup을 실제 API로 교정(`makeStubModel(4)`, `Bm25Index.build`, `SelectableBasicBackend(vecs)`, `ContentType.CODE`) — **expect 단언 무수정**. 진단 실행으로 right-reason 확인: findRelated 2건(시드 제외 후 companion 2건), 필터 search는 typescript 청크만. 게이트: `bunx tsc --noEmit | grep indexing/(index|dense).ts | grep -v TS5097` 비어있음(신규 타입 에러 0). `bun test src/indexing/` 격리 실행 시 T004 search/findRelated/stats 전부 green, 잔존 6 fail은 전부 범위 밖(save/load=T006/T007 throwing stub 3건, fromPath 에러메시지=T005 영역 2건, createIndexFromPath=create.test.ts 선존 1건 — stash 비교로 선존 확인). STOP(search/query API 구조 불일치) 미발동 — 가드를 CspIndex 레이어에 두고 selector 빈배열을 search.ts에 통과시키는 구조로 정합. commit ba30228
  - 주의: ESLint는 이 환경에서 `jiti` 미설치로 실행 불가(선존 인프라 이슈, 전 파일 공통) — 린트 게이트 미실행. 코드는 프로젝트 스타일(세미콜론 없음/단일 인용/2-space) 준수.
  - 주의: 전체 `bun test`는 샌드박스 tmpfs ENOSPC 플러딩으로 간헐 오염(stats/barrel/createIndexFromPath가 디스크풀로 추가 fail)이 관측됨 — 격리 실행에서는 재현 안 됨, 코드 회귀 아님.

- [x] (2026-06-18 17:30 KST) T005 CspIndex.fromGit 구현 — `mkdtempSync(tmpdir/csp-git-)` + `chmodSync 0o700`로 임시 체크아웃 생성, `git clone --depth 1`(ref 있으면 `--branch <ref>`, `--` 구분자 + `GIT_TERMINAL_PROMPT=0`로 비대화식·자격증명 프롬프트 차단, `spawnSync`)로 얕은 클론, 클론 루트를 그대로 `fromPath(dir, {ref 제외 옵션})`에 전달(파이프라인 재사용 — `.cspignore` 스캔이 클론 루트 기준 적용), **`finally`에서 `rmSync(recursive,force)` 정리**(성공/실패 모두). `cloneShallow` 헬퍼가 `result.error`(git 부재)·`status!==0`(클론 실패)에 stderr 포함 명확한 오류 throw. 시그니처 `(url:string, options?:CspIndexFromGitOptions)=>Promise<CspIndex>` 유지 → server.test.ts 정적 재할당 mock 호환. 테스트: 로컬 non-bare repo(`git init`+커밋)를 `file://`로 실제 얕은 클론 → 채워진 인덱스(1 file/sample.ts) + `csp-git-*` 임시 디렉터리 누수 0; 잘못된 `file://` URL → `/clone/i` 오류 + 누수 0. STOP(체크아웃이 `.cspignore` 스캔 범위 밖) 미발동 — `walkFiles(dir)`→`buildSpec(dir)`가 클론 루트의 ignore 파일을 읽으므로 클론 루트를 그대로 전달하면 규칙 보존. 게이트: typecheck `indexing/index(.test).ts` non-TS5097 신규 에러 0, 전체 `bun test` 353 pass/3 fail(baseline 351→353, 신규 실패 0, 잔존 3은 T006/T007 stub). server.test/cli.test 격리 green. commit 2399b1e
- [x] (2026-06-18 16:40 KST) T0A 선존 테스트-스위트 부채 정리 (RE-DISPATCH 후 완료) — 이전 라운드의 STOP은 모순 해결로 무효화됨: 오케스트레이터가 두 직렬화를 **별개 레이어**로 확정(types.ts=camelCase round-trip, search.ts toDict=snake_case wire). 따라서 항목 (b)는 "search.ts toDict 재사용"이 아니라 types.ts에 **독립적인** camelCase 헬퍼를 추가하는 것으로 재정의 → 모순 해소. 5개 항목 모두 처리:
  - (a) `mock.module` 누수 차단: Bun의 `mock.module`은 프로세스 전역·복원 불가(afterAll 재mock/`mock.restore` 둘 다 인접 파일로 누수 검증됨 → /tmp 프로브로 확인). DI seam으로 전환 — `CspIndex.fromPath/fromGit` **정적 메서드를 실 클래스 객체에 재할당**(server.ts가 import하는 동일 참조), `afterAll`에서 복원. stub은 빈 chunks의 **실 CspIndex 인스턴스** 반환(`instanceof CspIndex` + 빈인덱스 `search()===[]` 보존). server.ts 무수정. commit 49d17f5
  - (b) types.ts canonical camelCase 직렬화 헬퍼 추가(`chunkToDict`/`chunkFromDict`/`chunkLocation`/`searchResultToDict`/`ChunkDictInput`) — types.test.ts expect에 정확히 정렬(camelCase+location, null↔undefined, location strip, TypeError 가드). search.ts 무변경. `searchResultToDict`는 `{chunk,score}` 구조적 서브셋 수용(toDict 강제 안 함). commit b106e44
  - (c) enum 케이싱: types.test.ts/index.test.ts를 `CODE/DOCS/CONFIG`·`SEARCH/FIND_RELATED`로 정렬(문자열값 단언 유지). commit b106e44
  - (d) `Bm25Index.documents` read-only getter(per-doc 토큰수 배열, `.length===numDocs`). STOP(문서상태 부재) 미발동 — `#state.docLengths` 활용. commit a78e240
  - (e) cli.test.ts stub에 snake_case `toDict` 부여. commit aa74807
  게이트: 전체 `bun test` **351 pass / 3 fail / 0 error**(baseline 330/12/1에서 fail+error 13→3, 신규 실패 0). 잔존 3 fail은 전부 T006/T007 throwing stub(범위 밖). typecheck: 변경 소스(types.ts/sparse.ts) 신규 비-TS5097 에러 0, 전체 비-TS5097 에러 33→27 감소. ESLint는 jiti 미설치로 미실행(선존 인프라).
- [x] (2026-06-18 18:40 KST) T007 CspIndex.loadFromDisk(dir) 구현 — `dir`에서 인덱스 복원: (1) `existsSync(dir)` 없으면 `Index not found: <dir>` throw(테스트 `/Index not found/`); (2) manifest/chunks/bm25/vectors.bin/args.json 5개 아티팩트 누락 시 `Missing: <path>` throw(테스트 `/Missing:/`); (3) `manifest.schemaVersion !== INDEX_SCHEMA_VERSION`이면 `Index schema version mismatch: expected N, got M` throw; (4) chunks.json→`chunkFromDict` 매핑(camelCase round-trip, T006 chunkToDict와 무손실 대칭), `Bm25Index.load(dir)`, `SelectableBasicBackend.load(dir)`, `loadDenseModel(manifest.modelId)`로 모델 재로드; (5) `new CspIndex({model,bm25Index,semanticIndex,chunks,modelPath,root:manifest.sourceId,content:manifest.content})` 반환. **STOP 미발동** — chunkFromDict↔chunkToDict는 location strip 후 재계산으로 대칭(roundtrip 무손실), dense/bm25 load는 dir 기반으로 save와 시그니처 일치. 모델 dim 정합(아래 Surprises 참조): 재로드 stub 모델 dim(256)이 영속 벡터 dim과 다르면 `makeStubModel(semanticIndex.dim)`로 정렬해 query 재임베딩이 저장 백엔드와 비교가능하게 함(실 모델은 가중치로 dim 고정이라 무영향). 테스트: 기존 fail 3건(roundtrip persists / missing directory / missing artifact) green + 신규 2건(schema version mismatch throw / 무손실 roundtrip — chunks 동등 + stats 동등 + 2회 load 검색 결과 동일). 게이트: `bunx tsc --noEmit | grep indexing/index | grep -v TS5097` 0건(신규 타입 에러 0), 전체 `bun test` **363 pass / 0 fail / 0 error**(baseline 358/3/0 → loadFromDisk 3 fail green + 신규 2 test = 363 pass, fail 0). commit fc85f6b
- [x] (2026-06-18 20:30 KST) T008 cli `index -o` / `search·find-related --index`를 save/loadFromDisk에 배선(명시 경로 존중) — 배선은 이미 cli.ts에 존재했고(T003에서 `index.save(out)` / `CspIndex.loadFromDisk(p)` 참조 도입, T007에서 실구현 landing), T008은 **명시 경로 흐름이 종단으로 동작함을 검증**하고 회귀 방지 테스트로 고정. **소스 변경 0건**(cli.ts 무수정) — wiring이 이미 정확했음. cli.test.ts에 6개 테스트 추가: (1) `index -o <out>`이 명시 dir로 save(save 스파이 + 실제 manifest.json 생성); (2) `-o` 미지정 시 기존 `--out / -o is required` 오류 유지; (3) `search --index <p>` / (4) `find-related --index <p>`가 `readIndex`(loadFromDisk) seam으로 로드하며 **build 경로 미사용**(fromPath가 호출되면 throw하도록 주입해 증명); (5) 미존재 `--index` 경로 → 실 `loadFromDisk`가 `Index not found: <path>` 명확 오류 + exit 1; (6) **실 roundtrip(seam 없음)**: 작은 src dir를 `csp index <src> -o <out>`로 빌드·영속화→manifest.json 확인→`csp search --index <out>`로 재로드·검색 종단 동작. **두 STOP 모두 미발동** — search/find-related의 `--index`(loadFromDisk) vs build(fromPath/fromGit)는 상호배타 `if/else`(명시 경로 지정 시 build 경로 미실행, 충돌 없음); `_runIndex`의 `isGitUrl(path)?fromGit:fromPath` 분기는 search(cli.ts:423)와 동일 dispatch라 정합. mutation 검증: `--index` 존중 분기를 깨면(`if(false)`) 3건 fail, `-o` save dir을 틀리게 하면 2건 fail → 테스트가 실동작을 검증함 확인. 게이트: typecheck 신규 비-TS5097/80007 에러 0(잔존 cli.test.ts:197/226 TS2322·cli.ts:405 TS2379는 전부 선존 baseline, stash 비교 확인), 전체 `bun test` **369 pass / 0 fail / 0 error**(baseline 363 → +6 신규, 신규 실패 0). ESLint는 jiti 미설치로 미실행(선존 인프라). commit 5e39e95
- [x] (2026-06-18 02:47 KST) T006 CspIndex.save(dir) 구현 — `mkdirSync(dir,{recursive})` 후 5개 아티팩트 기록: `chunks.json`(`this.chunks.map(chunkToDict)` — camelCase round-trip, T0A 헬퍼 재사용), `bm25.json`(`this.bm25Index.save(dir)`), `vectors.bin`+`args.json`(`this.semanticIndex.save(dir)`), `manifest.json`(`{schemaVersion:INDEX_SCHEMA_VERSION=1, contentHash, sourceId:this.root, content:[...this.content], modelId:this.modelPath}`). `contentHash`는 직렬화 chunks JSON의 sha256(결정적; 정밀 repo-content hash는 T009 cache.ts). `INDEX_SCHEMA_VERSION`·`IndexManifest` export(T007이 검증·복원에 재사용). **두 STOP 모두 미발동** — (1) 파일명 충돌: manifest/chunks/bm25/vectors.bin/args.json 5개 상호 distinct(Bm25Index.save→bm25.json, SelectableBasicBackend.save→vectors.bin+args.json을 Read로 확인); (2) dense float drift: 프로브로 save→load roundtrip이 **bit-stable(maxDiff=0)** 임을 실측(아래 Surprises 참조) → 정규화 이중적용해도 NFR-002 동등성 유지, 즉흥 처리 불요. 게이트: typecheck `indexing/index(.test)` 신규 비-TS5097 에러 0, 전체 `bun test` **358 pass / 3 fail / 0 error**(baseline 353/3/0 → +5 save 테스트 green, fail 불변). 잔존 3 fail은 전부 T007 `loadFromDisk` stub(roundtrip은 save 성공 후 load에서 fail) — **T007 대기**. 신규 save 단독 테스트 5건(아티팩트 존재/디렉터리 생성/manifest 필드/chunkToDict 형식/결정적 hash) green. commit b659302

- **[T007] 재로드 모델 dim과 영속 벡터 dim 정합이 필요**: manifest.modelId로 `loadDenseModel`을
  재호출하면 stub 구현이 modelId와 무관하게 **고정 256-dim** 모델을 반환한다(dense.ts `_DEFAULT_STUB_DIM=256`).
  그러나 `index.test.ts`의 `buildIndex`는 `makeStubModel(4)` + 손수 만든 4-dim 벡터로 백엔드를 구성하므로,
  단순 재로드 시 `search`가 256-dim 쿼리 임베딩을 4-dim 저장 백엔드에 `query`해 dim mismatch throw.
  Evidence: `SelectableBasicBackend.query`가 `Query vector dimension mismatch` throw. 해소: load 시
  재로드 모델 dim이 `semanticIndex.dim`과 다르면 `makeStubModel(semanticIndex.dim)`로 정렬. 실 모델은
  가중치로 dim이 고정이고 `fromPath`는 동일 `loadModel`로 임베딩·쿼리해 항상 일치하므로 이 정렬은
  stub 시대 한정 무해한 보정이다. 실 파이프라인(fromPath)에서는 model.dim===backend.dim이라 분기 미발동.

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
- Decision: 두 청크 직렬화는 별개 레이어로 공존한다 — types.ts `chunkToDict`(**camelCase** + location,
  디스크/round-trip용)와 search.ts `SearchResult.toDict`(**snake_case** wire, CLI/MCP JSON용). 서로 재사용
  강제하지 않는다. `searchResultToDict`는 `{chunk,score}` 구조적 서브셋을 받아 `SearchResult.toDict` 클로저를
  요구하지 않는다(`SearchResult` 인터페이스는 `toDict` 필수 유지 → utils.ts:62 무영향).
  Rationale: T0A 항목 (b)의 초기 framing("search.ts toDict가 types.ts 헬퍼 재사용")은 두 형상이 상호배타적
  (camelCase vs snake_case)이라 구현 불가했고 이전 라운드 STOP의 원인이었음. 재정의로 모순 해소.
  Date/Author: 2026-06-18 / implement-executor
- Decision: server.test.ts의 CspIndex stub은 `mock.module`이 아니라 **정적 메서드 재할당 + afterAll 복원**으로
  구현한다(server.ts 무수정). Rationale: Bun 1.3.10의 `mock.module`은 프로세스 전역·복원 불가 — afterAll 재mock과
  `mock.restore()` 모두 인접 test 파일로 누수됨이 /tmp 프로브로 확인됨. 정적 메서드 재할당은 일반 객체 프로퍼티
  변경이라 복원 가능하고, 빈 chunks의 실 CspIndex 인스턴스를 반환해 `instanceof`/빈인덱스 동작을 보존한다.
  Date/Author: 2026-06-18 / implement-executor
- 플랜 리뷰(coherence/feasibility/completeness/scope-guardian/security/adversarial)에서
  create.ts 선존 컴파일 에러 3건·타입통합 toDict cascade·Phase A typecheck stub·캐시 권한 하드닝을
  반영(2026-06-17).

## Surprises & Discoveries

- create.ts의 Bm25Index API 불일치(컴파일 블로커) — Phase 탐색에서 발견, T002로 선반영.
- T002: 플랜의 "create.test.ts green" 목표는 T002 범위(create.ts 3-에러 교정)만으로 달성 불가 —
  create.test.ts가 **존재하지 않는/범위 밖 API**에 의존한다. baseline에서 이미 깨져 있던 에러임(전체
  스위트 baseline 3 errors 중 하나):
  1. `makeStubModel`이 dense.ts에서 export 안 됨 + 테스트는 `makeStubModel('test-model', 4)`/`makeStubModel()`
     호출하나 실제 시그니처는 미export `makeStubModel(dim: number)`.
  2. `bm25Index.documents` 접근 — Bm25Index는 `documents` 프로퍼티를 노출하지 않음(상태는 private `#state`).
  3. 테스트가 `ContentType.Docs` 사용(→ `ContentType.DOCS`여야 함).
  추가로 create.ts에 **T002 범위 밖 선존 타입 에러 4건** 잔존(증가 아님, baseline 동일): line 49 `walkFiles`
  AsyncIterable를 `for...of`로 소비, line 67 `chunkSource`가 `Promise<Chunk[]>` 반환을 동기 spread,
  line 67 `detectLanguage` `string|undefined` vs `string|null`, line 74 dense.ts 로컬 `Chunk` 타입 불일치.
  이 4건은 async/배선 이슈로 **T003(src/indexing/index.ts 오케스트레이션)** 영역이며 createIndexFromPath의
  런타임 동작에도 영향. create.test.ts를 green으로 만들려면 dense.ts(makeStubModel export)·sparse.ts
  (documents 접근자)·create.test.ts(ContentType.DOCS)·create.ts(async 배선) 교차 수정 필요 — 모두 T002
  Files 범위(src/indexing/create.ts) 밖이므로 T002에서 처리하지 않음. T003 착수 시 함께 해소 권장.
- search.ts 타입 중복(`TODO(integration)`) — 배선 전 통합 필요(T001).
- mcp IndexCache가 이미 오케스트레이터에 의존 — 디스크 캐시 정합화 필요(T012).
- T001: `../tokens.ts`의 `tokenize`는 search.ts 로컬 tokenize와 동작 동등(동일 TOKEN_RE/CAMEL_RE/
  splitIdentifier 로직; tokens.ts는 순수 소문자 토큰 fast-path만 추가하나 출력 동일). 교체 후
  `bun test src/search.test.ts` 24 pass(기존 20 + toDict 4), 전체 스위트 fail 집합 불변(316→320 pass,
  5 fail/3 errors 동일) — 토크나이저 회귀 없음 확인.
- T003: 플랜 시나리오의 "index.test.ts fromPath 테스트 green" 목표는 **T003 Files 범위
  (src/indexing/index.ts 단일)만으로 달성 불가** — `index.test.ts`가 모듈 최상단에서 import하는
  심볼 중 일부가 T003 범위 밖 모듈에 있어 테스트 파일이 **로드조차 되지 않는다**(SyntaxError, 3 errors
  중 하나, baseline부터 깨짐). T003 후 `DEFAULT_CONTENT`(index.ts) 미export 블로커는 해소됐고
  로더가 다음 블로커로 진행했으나, 잔존 블로커는 전부 범위 밖:
  1. `makeStubModel`이 `dense.ts`에서 export 안 됨 + 테스트는 `makeStubModel('test-model', 4)` 호출하나
     실제 시그니처는 미export `makeStubModel(dim: number)` → **dense.ts** 수정 필요(T003 범위 밖).
  2. 테스트가 `new SelectableBasicBackend(vectors, 4)`(2번째 인자=dim) 사용하나 현 ctor는
     `(vectors, options: BasicArgs)` → **dense.ts** 수정 필요.
  3. 테스트가 `new Bm25Index(chunks.map(() => ['x']))`(public ctor) 사용하나 `Bm25Index` ctor는
     `private`(only `static build`) → **sparse.ts** 수정 필요.
  4. 테스트가 `ContentType.Code`(line 197) 사용 — enum은 `CODE`(대문자) → **테스트 파일** 수정 필요.
  index.ts 측 in-scope 계약(생성자 옵션 객체 형상, stats, fromPath, DEFAULT_CONTENT, save/loadFromDisk
  stub, loadModel 튜플)은 모두 충족했고 테스트의 `buildIndex({...})`/`stats`/`fromPath` 기대 형상과 일치.
  search/findRelated의 filterLanguages/filterPaths·findRelated 동작 단언은 **T004 behavioral 영역**이라
  T003 stub에서는 의도적으로 미충족. 따라서 `index.test.ts` green 전환은 dense.ts(makeStubModel export +
  SelectableBasicBackend(vectors,dim) ctor)·sparse.ts(Bm25Index public ctor)·테스트(ContentType.CODE)
  교차 수정과 T004 동작 배선이 함께 done돼야 가능 — 테스트 약화 금지 원칙에 따라 T003에서 범위 확장하지 않음.
  게이트는 "index.ts 신규 에러 0 + 전체 스위트 baseline 불변"으로 충족.
- T004: index.test.ts setup 정렬에 필요한 ctor/factory 교정은 T003이 예측한 dense.ts/sparse.ts 교차 수정 중
  **dense.ts(makeStubModel export)와 SelectableBasicBackend ctor**만 필요했고, `Bm25Index.build`·
  `SelectableBasicBackend(vecs)`는 이미 실제 API와 일치(테스트가 추측한 `new Bm25Index(...)`/`(vecs,4)`가
  틀렸던 것) — 테스트 setup만 실제 API로 교정하면 됐고 sparse.ts public ctor 추가는 불필요했다. T004 Files에
  sparse.ts가 없었던 이유와 정합(index.test.ts는 `Bm25Index.build`/`SelectableBasicBackend(vecs)`로 교정).
- T004: STOP 조건(search.ts API가 index.test.ts 기대와 구조 불일치) 미발동 — search.ts의 selector 시맨틱이
  빈 selector를 `effectiveK=0`→`[]`로 처리하므로, 빈쿼리/topK<=0/빈필터 가드를 **CspIndex 레이어**에 두면
  구조적으로 정합. search.ts 자체는 빈쿼리에 semantic 결과를 반환하므로(BM25만 토큰 0→[]) blank-query [] 보장은
  CspIndex.search의 `query.trim().length===0` 가드가 담당. 이 분담이 STOP을 회피한 핵심.
- 환경: ESLint가 `jiti` 미설치로 이 워크트리에서 실행 불가(전 파일 공통, 선존). bun:test 전체 실행 시 샌드박스
  tmpfs가 ENOSPC로 차서 무관한 테스트(stats/barrel/createIndexFromPath)가 디스크풀로 추가 fail하는 오염 관측 —
  격리 실행(`bun test src/indexing/`)에서는 재현 안 됨. 후속 executor는 전체 스위트 카운트보다 격리 실행을 신뢰할 것.
- **[오케스트레이터 정정 진단, 2026-06-18]** 전체 스위트 추가 fail의 진짜 원인은 tmpfs ENOSPC가 아니라
  **`src/mcp/server.test.ts:57`의 `mock.module('../indexing/index.ts', ...)` 전역 누수**다(디스크 148Gi 여유 확인).
  server.test.ts가 모듈 로드 시점에 CspIndex를 MockedCspIndex로 교체하고 복원(afterAll/restore)이 없어, 같은
  프로세스에서 뒤에 도는 `indexing/index.test.ts`가 mocked stub을 받아 save/loadFromDisk/stats/fromPath-guard가
  전부 사라진다(→ "loadFromDisk is not a function", fromPath가 resolve, stats undefined). **격리 실행에선 전부 통과.**
- **선존 스캐폴드 테스트 부채(이 트랙 plan 범위 밖, 본 작업이 노출시킴)**: 포팅된 실제 소스와 다른 API를 가정한 테스트들 —
  (a) `src/index.test.ts`(barrel) `ContentType.Code` (enum은 `CODE`); (b) `src/types.test.ts`가 미존재 export
  `searchResultToDict`/`chunkToDict`/`chunkFromDict`/`chunkLocation` import + `ContentType.Code/Docs/Config`;
  (c) `src/indexing/create.test.ts`가 미존재 `bm25Index.documents` accessor 단언; (d) `src/cli.test.ts`의
  stub-mock이 `toDict` 미제공 → `csp search JSON` fail. 모두 "테스트를 실제 소스 API에 정렬" 또는 "소스에 해당 API 추가"
  결정 필요 — 단일 태스크로 깔끔히 안 떨어지는 구조적 이슈라 사용자 판단 요청(BLOCK).
- **Phase A 기능 구현(T001~T004 + fromPath 가드)은 격리 실행 기준 모두 정상**: search.test 24 pass,
  indexing/index.test 격리 11 pass(나머지 3은 T006/T007 미구현 throwing stub). typecheck는 변경 소스에 신규 에러 0
  (TS5097 .ts 확장자 baseline 제외).
- **[T0A] Bun 1.3.10 `mock.module`은 비가역적이다**: 한번 적용하면 같은 프로세스의 **모든** 후속 import에
  적용되며 되돌릴 수 없다. `afterAll(() => mock.module(path, () => realModule))` 재mock도, `mock.restore()`도
  인접 test 파일로 누수가 유지됨 — `/tmp` 최소 프로브 2건으로 확인(restore 후에도 두 번째 파일이 여전히 mock 관찰).
  Evidence: `bun test fileA fileB`에서 fileA의 top-level `mock.module(dense)` → fileB의 `makeStubModel`이
  복원 시도 후에도 `'MOCKED'` 반환. 결론: 모듈 단위 stub이 필요하면 `mock.module` 대신 DI seam(정적 메서드/주입)을
  써야 누수가 없다. 또한 Bun은 모든 test 파일의 top-level을 테스트 실행 전에 평가하므로, 파일 실행 순서를 바꿔도
  top-level `mock.module` 누수는 회피되지 않는다(server↔indexing 양방향 모두 8 fail 관측).
- **[T0A] 선존 스캐폴드 부채 정리 완료**: 위 BLOCK 항목 (a)~(e)는 RE-DISPATCH(모순 해결)로 전부 해소.
  전체 스위트 330/12/1 → 351/3/0. 잔존 3 fail은 T006/T007 throwing stub(범위 밖).
- **[T008] cli 명시-경로 배선은 소스 변경 0건으로 종단 동작** — T003이 `index.save(out)`/`CspIndex.loadFromDisk(p)`
  참조를 (당시 throwing stub인) API에 미리 도입했고 T007이 실구현을 landing했으므로, T008 시점엔 wiring이 이미
  정확했다. 따라서 T008은 GREEN(소스 수정) 없이 **검증 + 회귀 방지 테스트**만으로 완료. `--index`(loadFromDisk)와
  build(fromPath/fromGit)는 상호배타 `if/else`라 "명시 경로 우회" STOP은 구조적으로 발동 불가. Evidence: `git diff
  --stat`이 cli.test.ts 1파일만, cli.ts 무변경; mutation(분기 무력화·save dir 오염)으로 신규 테스트가 fail함을 실측
  (각 3건/2건). 후속 T011(자동 캐시 배선)은 이 상호배타 구조 위에 `--index` 미지정 분기만 `loadOrBuildIndex`로 바꾸면
  되며, 명시 경로 분기는 건드리지 않아야 T008 보장이 유지된다.
- **[T006] dense save→load roundtrip은 float drift 없이 bit-stable** (STOP-2 fact-based 판정): `SelectableBasicBackend`
  생성자가 벡터를 in-place L2 정규화 → `save`는 이미 정규화된 벡터를 `vectors.bin`(Float32)으로 기록 → `load`가
  생성자를 통해 **재정규화**(unit-length의 재정규화는 `/≈1.0`로 멱등). 격리 프로브로 실측: 미정규화 입력으로 빌드한
  b1을 save→load한 b2와 `maxDiff(b1.vectors, b2.vectors)=0`, 2차 roundtrip(b2→b3)도 `maxDiff=0`, query 랭킹
  `[[2,...],[1,...],[0,...]]` 동일. Evidence: `bun test`(임시 프로브, 커밋 안 함). **결론: NFR-002 roundtrip 동등성
  위험 없음 → 미정규화 저장/skipNormalize 같은 즉흥 처리 불요.** T007 `loadFromDisk`는 `SelectableBasicBackend.load`를
  그대로 재사용해도 안전(재정규화가 등가성을 깨지 않음). `vectors.bin`/`args.json`/`bm25.json`/`chunks.json`/`manifest.json`
  5개 파일명이 상호 distinct임도 Read로 확인 → STOP-1(파일명 충돌)도 미발동.
- **[T009] upstream semble에 디스크 캐시 `cache.py`가 없다** (STOP gate fact): 캐시된 체크아웃
  `~/.ask/github/github.com/MinishLab/semble/main`(May-27 baseline, `eacbe43` 이전)에 `cache.py` 부재.
  유일한 "cache"는 `mcp.py`의 인메모리 `_IndexCache`(LRU `_CACHE_MAX_SIZE=10`, **소스 경로/URL 키**, 세션
  한정) + Python `functools.cache` 메모이즈(`dense._load_cached`, `chunking._cached_get_parser`)뿐. 디스크
  content-hash 캐시 디렉터리 키 모델은 upstream에 **존재하지 않음** — 글로벌 `~/.csp/index/<key>` 자동 캐시는
  upstream #162(글로벌 cache auto-indexing, **미포팅** — `upstream-semble-sync-baseline` 메모리 확인)에
  대응하는 csp-original 설계다. Evidence: `find .../semble/main -iname cache.py` 무결과;
  `grep -rE "content_hash|cache_dir|cache_key"` src/semble → 인메모리 LRU·functools만 매칭. **결론: T009 STOP
  (upstream 키 모델과 근본 충돌) 미발동** — 충돌할 upstream 디스크 캐시 모델이 없음. 인메모리 캐시가 "소스 동일성"으로
  키잉하는 점은 plan의 source-identity 컴포넌트와 정합. T013 ADR은 이 divergence(upstream 인메모리-only ↔ csp
  디스크 content-hash)를 기록해야 한다. (주의: `ask` CLI는 비대화식 셸 PATH에 없고 tmpfs가 ENOSPC라
  `CLAUDE_CODE_TMPDIR=/Users/lms/.cache/csp-tmp`로 우회; 전체 `bun test`도 이 tmpdir에서 384 green 클린 실행.)
- [x] (2026-06-18 22:10 KST) T009 cache 모듈 신규 — `src/indexing/cache.ts`: `resolveCacheDir(source, content, {baseDir?, ref?})` → `<home>/index/<key>`(key=sha256({sourceId, content정렬, ref}) 32자 절단; 로컬경로 `normalize`/URL verbatim), `computeContentHash(files)`(path 정렬→length-prefixed path+bytes 순차 sha256; string·Uint8Array 동등), `ensureCacheDir(dir,{baseDir?})`(`mkdir {recursive,mode:0o700}` + home→index→leaf 체인 각각 `chmodSync 0o700`로 기존 디렉터리 보정 — recursive mkdir이 기존 dir 권한 미변경 보완, NFR-003). **테스트 격리**: `baseDir` 주입 옵션으로 실 `~/.csp` 미오염(테스트는 `mkdtempSync` tmp home 사용). **STOP(upstream cache.py 키 모델 충돌) 미발동** — 캐시된 upstream 체크아웃(`~/.ask/.../MinishLab/semble/main`, May-27 baseline)에 **cache.py 자체가 없음**: 디스크 content-hash 캐시 키 모델 부재(`mcp.py`의 인메모리 `_IndexCache` LRU=소스경로 키 + `functools.cache` 메모이즈만). 글로벌 `~/.csp/` 캐시는 upstream #162(미포팅)에 해당하는 csp-original 설계라 충돌할 upstream 모델이 없음(plan Architecture Decision도 이를 명시, T013 ADR에서 근거화 예정). **STOP(fromGit content-hash 비결정성) 미발동** — T009는 순수함수이고 입력만 결정적이면 됨; 입력 수집은 T010. 게이트: `bunx tsc --noEmit | grep indexing/cache | grep -vE "TS5097|TS80007"` 0건, 전체 `bun test` **384 pass / 0 fail / 0 error**(baseline 369 + 신규 15). commit 2236cce
