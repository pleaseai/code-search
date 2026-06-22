# Changelog

## [0.1.3](https://github.com/pleaseai/code-search/compare/v0.1.2...v0.1.3) (2026-06-22)


### Bug Fixes

* **release:** set GH_REPO so upload-release-assets can find the repo ([#48](https://github.com/pleaseai/code-search/issues/48)) ([dcfd873](https://github.com/pleaseai/code-search/commit/dcfd87350df7ee01b8df47b970d739234b9c0fdf))

## [0.1.2](https://github.com/pleaseai/code-search/compare/v0.1.1...v0.1.2) (2026-06-22)


### Bug Fixes

* **release:** build musl via cargo-zigbuild + make release resilient ([#46](https://github.com/pleaseai/code-search/issues/46)) ([ec7fbb3](https://github.com/pleaseai/code-search/commit/ec7fbb3b82d32e29ec9635066ad7d5f2b4d5a5ac))

## [0.1.1](https://github.com/pleaseai/code-search/compare/v0.1.0...v0.1.1) (2026-06-22)


### Bug Fixes

* **release:** sync Cargo.lock on release so --locked builds pass ([#43](https://github.com/pleaseai/code-search/issues/43)) ([257378c](https://github.com/pleaseai/code-search/commit/257378c0ca1ebcfca7615be3283b53ac95b1f3a1))

## 0.1.0 (2026-06-20)


### Features

* Claude Code · Codex 플러그인 추가 ([#19](https://github.com/pleaseai/code-search/issues/19)) ([c86fb07](https://github.com/pleaseai/code-search/commit/c86fb079e6b07961207ffa541463fa092c47d081))
* **cli:** port subcommands (search/index/find-related/init/savings/mcp) from semble ([#15](https://github.com/pleaseai/code-search/issues/15)) ([2042846](https://github.com/pleaseai/code-search/commit/2042846453696a582b3a5ded1209d2f7b76ebeff))
* **csp:** expand tree-sitter coverage via tree-sitter-language-pack ([#39](https://github.com/pleaseai/code-search/issues/39)) ([9a0cde3](https://github.com/pleaseai/code-search/commit/9a0cde3c492fb9dae8619810299014fe331807ff))
* **deps:** bootstrap external dependencies and adr ([#6](https://github.com/pleaseai/code-search/issues/6)) ([0d930fd](https://github.com/pleaseai/code-search/commit/0d930fdd9429dd0f0142cff74bd3563c7173a7ab))
* distribute csp via Homebrew tap ([#22](https://github.com/pleaseai/code-search/issues/22)) ([0278323](https://github.com/pleaseai/code-search/commit/0278323a7e5d10e49213f3400702d69f4f376624))
* **indexing:** port BM25 enrichment + index from semble ([#9](https://github.com/pleaseai/code-search/issues/9)) ([d1692bd](https://github.com/pleaseai/code-search/commit/d1692bd3e5a1a5ce345a402d7fe746e1394a28d5))
* **indexing:** port CspIndex orchestrator (fromPath/fromGit/search/findRelated/save/load) ([#17](https://github.com/pleaseai/code-search/issues/17)) ([df14647](https://github.com/pleaseai/code-search/commit/df146475cf0d2dc37d7770c85735b0af3c6ec6a2))
* **indexing:** port extension→language detection from semble ([#3](https://github.com/pleaseai/code-search/issues/3)) ([3c74752](https://github.com/pleaseai/code-search/commit/3c74752096d81a56db8022a834ea4ecf36682a16))
* **indexing:** port gitignore-aware file walker from semble ([#8](https://github.com/pleaseai/code-search/issues/8)) ([d0b1ec5](https://github.com/pleaseai/code-search/commit/d0b1ec5f934688f847e97c423319a9a8b44fba03))
* **indexing:** port Model2Vec embedding + vector backend from semble ([#7](https://github.com/pleaseai/code-search/issues/7)) ([3937a3f](https://github.com/pleaseai/code-search/commit/3937a3f859bc25a489004a67fdbdecfe019c2cba))
* **index:** public library barrel re-exporting CspIndex + types ([#12](https://github.com/pleaseai/code-search/issues/12)) ([a4931d8](https://github.com/pleaseai/code-search/commit/a4931d84159872a38a1dca2dab7ea6536f451ab3))
* **mcp:** port MCP server with search/find_related tools from semble ([#16](https://github.com/pleaseai/code-search/issues/16)) ([18110e6](https://github.com/pleaseai/code-search/commit/18110e6a42a999c49951ea38435c091e92b91aa5))
* **ranking:** port path penalties + rerankTopK from semble ([#4](https://github.com/pleaseai/code-search/issues/4)) ([df248d2](https://github.com/pleaseai/code-search/commit/df248d2d7fd68f0f2839948a7886ef5868cf4a80))
* **ranking:** port weighting + boosting from semble ([#13](https://github.com/pleaseai/code-search/issues/13)) ([08f5b06](https://github.com/pleaseai/code-search/commit/08f5b06d9302d794ecac110d971c4638b33c6f63))
* **search:** port hybrid RRF + alpha-blend pipeline from semble ([#14](https://github.com/pleaseai/code-search/issues/14)) ([97b6415](https://github.com/pleaseai/code-search/commit/97b6415e3a3098fb0cb7a39a05a7f788efede557))
* **search:** wire real ranking pipeline + reconcile chunk size to 750 ([#37](https://github.com/pleaseai/code-search/issues/37)) ([3b47fb5](https://github.com/pleaseai/code-search/commit/3b47fb5d229bb866cf7686fab6324506c58758f3))
* **stats:** port savings.jsonl telemetry from semble ([#11](https://github.com/pleaseai/code-search/issues/11)) ([2585b6b](https://github.com/pleaseai/code-search/commit/2585b6bca30cb52ced736544ee22705138aac30c))
* sync recent upstream semble changes (savings output, agents, clear) ([#20](https://github.com/pleaseai/code-search/issues/20)) ([d1ff3f6](https://github.com/pleaseai/code-search/commit/d1ff3f6c05eed3e465d325884a355e3e4f634eb2))
* **tokens:** port identifier-aware tokenizer from semble ([#1](https://github.com/pleaseai/code-search/issues/1)) ([e619bc2](https://github.com/pleaseai/code-search/commit/e619bc2833601cd0231bca8b74f585f97b576c12))
* **types:** port Chunk/SearchResult/ContentType from semble ([#5](https://github.com/pleaseai/code-search/issues/5)) ([94ba6aa](https://github.com/pleaseai/code-search/commit/94ba6aa07e2dd0e698a3d4ce11fbbd3a4de3217d))
* **utils:** port isGitUrl/resolveChunk/formatResults from semble ([#2](https://github.com/pleaseai/code-search/issues/2)) ([7a15929](https://github.com/pleaseai/code-search/commit/7a15929b251e938355695b6f96093fa4514b8258))
* wire up CspIndex orchestrator + global index cache ([#18](https://github.com/pleaseai/code-search/issues/18)) ([#21](https://github.com/pleaseai/code-search/issues/21)) ([da945a5](https://github.com/pleaseai/code-search/commit/da945a5f77214f19a1a1a9840a3043fdd9114f39))


### Bug Fixes

* **chunking:** enable AST chunking by wiring real ALL_LANGUAGES ([#28](https://github.com/pleaseai/code-search/issues/28)) ([#31](https://github.com/pleaseai/code-search/issues/31)) ([900de35](https://github.com/pleaseai/code-search/commit/900de35705a8011e51d10947e0a44764b60afea9))
* **search:** wire ranking modules (query boosts + path penalties) ([#27](https://github.com/pleaseai/code-search/issues/27)) ([#32](https://github.com/pleaseai/code-search/issues/32)) ([94d61f9](https://github.com/pleaseai/code-search/commit/94d61f9930678ba8692f01d1f443f34490a6e823))


### Documentation

* add ARCHITECTURE.md ([8c08597](https://github.com/pleaseai/code-search/commit/8c085973eebb21665a36ea18a3193f8b2b835da4))
* add semble→Rust reference analysis under .please/docs/references ([#35](https://github.com/pleaseai/code-search/issues/35)) ([6e31708](https://github.com/pleaseai/code-search/commit/6e3170813e5e2a251f4e33f717b0c00a3f33584b))
* **references:** add cocoindex-code and model2vec reference analyses ([#36](https://github.com/pleaseai/code-search/issues/36)) ([beae45d](https://github.com/pleaseai/code-search/commit/beae45d5a30b238371d457d9e7c2385d7f21dfd4))
* remove stale stub/TODO comments on implemented modules ([#29](https://github.com/pleaseai/code-search/issues/29)) ([#30](https://github.com/pleaseai/code-search/issues/30)) ([cb5b584](https://github.com/pleaseai/code-search/commit/cb5b584fc7e3ffe4e113eaaa9c3da47bdcdaab3e))
