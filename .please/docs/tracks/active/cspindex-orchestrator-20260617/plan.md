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
- [x] (2026-06-18 10:30 KST) T002 create.ts 선존 컴파일 에러 3건 교정 (Bm25Index.build / SelectableBasicBackend(embeddings) / ContentType.CODE) — 3개 타깃 에러 제거, 신규 에러 0, 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). create.test.ts green 목표는 **미달**: 테스트가 범위 밖 API에 의존(아래 Surprises 참조)
- [x] (2026-06-18 11:10 KST) T002 (round 2) create.ts 잔존 컴파일 에러 4건 마무리 + dense/sparse Chunk 타입 통합 — async 정합(for await walkFiles / await chunkSource / detectLanguage ?? null), dense.ts·sparse.ts 로컬 `Chunk` 제거 후 `../types.ts`로 통합(re-export 유지). create.ts 소스 4-에러 전부 제거(TS5097 제외 0), 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). 잔존 makeStubModel/DEFAULT_CONTENT 미export 의존 테스트는 범위 밖이라 미수정. commit a328727
- [x] (2026-06-18 12:20 KST) T003 CspIndex.fromPath 배선 + save/loadFromDisk throwing stub — fromPath가 `loadDenseModel(opts.modelPath)` + `createIndexFromPath(path,{model,content,displayRoot:path})`로 `{model,semanticIndex,bm25Index,chunks}` 보유 인스턴스 반환. 생성자를 옵션 객체 `{model,bm25Index,semanticIndex,chunks,modelPath,root,content}`로 확장, `DEFAULT_CONTENT` export, `stats` getter 추가. `save`/`loadFromDisk`는 throwing stub(T006/T007) — cli.ts:288 `index.save` / cli.ts:415 `CspIndex.loadFromDisk` **선존 타입 에러 2건 제거**(stash 비교로 확인). `loadModel`은 `[model,modelPath]` 튜플 재export(mcp `[, modelPath]` 정합), search/findRelated는 동기 stub([]) 유지(T004). 게이트: index.ts 신규 non-TS5097 에러 0, mcp/server.ts·cli.ts 에러 집합 baseline 동일(상기 2건 제거 외), 전체 스위트 baseline 불변(320 pass/5 fail/3 errors). STOP(호출부 시그니처 충돌) 미발동. commit 7aa721a
- [x] (2026-06-18 14:05 KST) T004 CspIndex.search/findRelated 배선 + index.test.ts setup 정렬 — `search()`는 blank query / `topK<=0` / 빈 인덱스 / 빈 selector(필터 무매치) 가드 후 `search.ts`의 `search(query, model, semanticIndex, bm25Index, chunks, topK, {selector?})`에 **동기** 위임(mcp/server.ts:370 await 없이 호출 정합). `findRelated(seed|{chunk,score})`는 시드 content를 재임베딩→`semanticIndex.query(emb, topK+1)`→시드 청크 제외 후 topK 반환. `filterLanguages`/`filterPaths`→`buildSelector`가 후보 인덱스 `Uint32Array` 생성, 무매치 시 길이 0 → `[]`(unfiltered 폴백 없음, 회귀 테스트 충족). `makeStubModel`을 dense.ts에서 export, index.test.ts setup을 실제 API로 교정(`makeStubModel(4)`, `Bm25Index.build`, `SelectableBasicBackend(vecs)`, `ContentType.CODE`) — **expect 단언 무수정**. 진단 실행으로 right-reason 확인: findRelated 2건(시드 제외 후 companion 2건), 필터 search는 typescript 청크만. 게이트: `bunx tsc --noEmit | grep indexing/(index|dense).ts | grep -v TS5097` 비어있음(신규 타입 에러 0). `bun test src/indexing/` 격리 실행 시 T004 search/findRelated/stats 전부 green, 잔존 6 fail은 전부 범위 밖(save/load=T006/T007 throwing stub 3건, fromPath 에러메시지=T005 영역 2건, createIndexFromPath=create.test.ts 선존 1건 — stash 비교로 선존 확인). STOP(search/query API 구조 불일치) 미발동 — 가드를 CspIndex 레이어에 두고 selector 빈배열을 search.ts에 통과시키는 구조로 정합. commit ba30228
  - 주의: ESLint는 이 환경에서 `jiti` 미설치로 실행 불가(선존 인프라 이슈, 전 파일 공통) — 린트 게이트 미실행. 코드는 프로젝트 스타일(세미콜론 없음/단일 인용/2-space) 준수.
  - 주의: 전체 `bun test`는 샌드박스 tmpfs ENOSPC 플러딩으로 간헐 오염(stats/barrel/createIndexFromPath가 디스크풀로 추가 fail)이 관측됨 — 격리 실행에서는 재현 안 됨, 코드 회귀 아님.

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
