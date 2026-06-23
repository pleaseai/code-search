<h2 align="center">
  csp — Code Search Please<br/>
  에이전트를 위한 빠르고 정확한 코드 검색<br/>
  <sub>grep+read 대비 약 98% 적은 토큰 사용</sub>
</h2>

<div align="center">
  <h2>
    <a href="https://www.npmjs.com/package/@pleaseai/csp"><img src="https://img.shields.io/npm/v/@pleaseai/csp?color=%23007ec6&label=npm" alt="npm version"></a>
    <a href="https://crates.io/crates/code-search-please"><img src="https://img.shields.io/crates/v/code-search-please?color=%23dea584&label=crates.io" alt="crates.io version"></a>
    <a href="https://codecov.io/gh/pleaseai/code-search"><img src="https://img.shields.io/codecov/c/github/pleaseai/code-search?label=coverage" alt="Coverage"></a>
    <a href="https://sonarcloud.io/summary/new_code?id=pleaseai_code-search"><img src="https://sonarcloud.io/api/project_badges/measure?project=pleaseai_code-search&metric=alert_status" alt="Quality Gate Status"></a>
    <a href="https://socket.dev/npm/package/@pleaseai/csp"><img src="https://socket.dev/api/badge/npm/package/@pleaseai/csp" alt="Socket Badge"></a>
    <a href="https://github.com/pleaseai/code-search/blob/main/LICENSE">
      <img src="https://img.shields.io/badge/license-MIT-green" alt="License - MIT">
    </a>
  </h2>

[English](./README.md) | 한국어

[퀵스타트](#퀵스타트) •
[MCP 서버](#mcp-서버) •
[AGENTS.md](#agentsmd) •
[CLI](#cli) •
[동작 원리](#동작-원리)

</div>

> **Rust 포트.** `csp`는 Python으로 작성된 [MinishLab/semble](https://github.com/MinishLab/semble)의 Rust 포트입니다. 알고리즘과 설계의 모든 공로는 원저자에게 있습니다. Rust 바이너리는 Homebrew와 npm으로 배포되는 자체 완결형 실행 파일입니다(Homebrew 빌드는 런타임이 필요 없음).

`csp`는 에이전트를 위해 설계된 코드 검색 라이브러리입니다. 필요한 코드 스니펫을 즉시 반환하며, `grep` + `read` 조합 대비 약 **98% 적은 토큰**만 사용합니다. 전체 코드베이스의 인덱싱과 검색이 1초 이내에 끝나며, API 키, GPU, 외부 서비스 없이 CPU 위에서만 동작합니다. [MCP 서버](#mcp-서버)로 실행하거나 [AGENTS.md](#agentsmd) 안내로 셸에서 호출하면 Claude Code, Cursor, Codex, OpenCode 등 어떤 에이전트라도 임의의 리포지토리에 즉시 접근할 수 있습니다.

## 퀵스타트

에이전트는 `"인증은 어떻게 처리되나요?"` 같은 자연어로 `csp`에 질의하고, 관련 코드 스니펫만 받아봅니다. grep 하거나 전체 파일을 읽을 필요가 없습니다.

`csp`는 세 가지 보완적인 사용 방법을 제공합니다. 셋 다 함께 쓰는 것을 권장하지만, 필요에 따라 골라 쓰셔도 됩니다.

- **[MCP 서버](#mcp-서버)**: 에이전트가 호출할 수 있는 MCP 서버.
- **[AGENTS.md](#agentsmd)**: CLI 호출 지침을 담은 `AGENTS.md` 스니펫.
- **[서브 에이전트](#서브-에이전트-설정)**: 지원 하니스를 위한 전용 `csp-search` 서브 에이전트.

### MCP

`csp`를 MCP 네이티브 도구로 노출시켜 에이전트가 직접 호출하게 합니다. Claude Code에 다음과 같이 추가합니다.

```bash
claude mcp add csp -s user -- bunx @pleaseai/csp mcp
```

다른 하니스(Cursor, Codex, OpenCode 등)는 아래 [MCP 서버](#mcp-서버) 섹션을 참고하세요.

### 플러그인 (Claude Code & Codex)

`csp`를 설정하는 가장 빠른 방법은 플러그인입니다. MCP 서버와 `csp-search` 헬퍼를 한 번에 등록하며, 동일한 디렉터리에서 Claude Code와 Codex 양쪽을 지원합니다.

Claude Code:

```text
/plugin marketplace add pleaseai/code-search
/plugin install csp@pleaseai
```

Codex:

```bash
codex plugin marketplace add pleaseai/code-search
codex plugin add csp@pleaseai
```

`csp` MCP 서버(`search`, `find_related`)와 함께 `csp-search` 서브에이전트(Claude Code) / 스킬(Codex)을 번들합니다. 서버를 `bunx`로 실행하려면 `PATH`에 [Bun](https://bun.sh)이 있어야 합니다. 자세한 내용은 [plugins/csp](plugins/csp/README.md)를 참고하세요.

### AGENTS.md

에이전트의 컨텍스트에 `csp` 사용법을 추가해 언제 어떻게 CLI를 호출할지 알 수 있도록 합니다. 먼저 `csp` CLI를 설치한 뒤, 아래 스니펫을 `AGENTS.md` 또는 `CLAUDE.md`에 추가하세요.

```bash
# Homebrew (macOS / Linux) — Node/Bun 없이 동작하는 독립 실행 바이너리
brew install pleaseai/tap/csp

# 또는 JavaScript 패키지 매니저로 설치 (PATH에 Bun 또는 Node 22+ 필요)
bun add -g @pleaseai/csp     # bun으로 설치 (권장)
npm install -g @pleaseai/csp # 또는 npm
pnpm add -g @pleaseai/csp    # 또는 pnpm
```

> Homebrew formula는 자체 완결형 Rust 바이너리를 제공합니다(`cargo build --release`; tree-sitter 문법·임베딩 런타임 내장)므로 런타임에 Node/Bun이 필요 없습니다. npm 패키지는 동일한 바이너리를 작은 Node 런처 뒤에 담아 배포하므로, `npm`/`bun`/`pnpm` 설치 경로는 `PATH`에 Bun 또는 Node 22+가 필요합니다. 인덱스는 `~/.csp/`에 캐시됩니다([ADR 0002](.please/docs/decisions/0002-index-storage-cache-model.md) 참고).

<details>
<summary>AGENTS.md / CLAUDE.md 스니펫</summary>

````markdown
## Code Search

코드를 grep 으로 찾기 전에 `csp search`를 사용하세요. 동작을 자연어로 설명하거나 심볼/식별자명을 입력하면 됩니다.

```bash
csp search "authentication flow" ./my-project
csp search "saveCheckpoint" ./my-project
csp search "save model to disk" ./my-project --top-k 10
```

여러 번 검색할 예정이라면 `csp index`로 인덱스를 만들어 두세요.

```bash
csp index ./my-project -o my_index
```

이후 인덱스를 재사용할 수 있습니다.

```bash
csp search "saveCheckpoint" --index my_index
```

인덱스는 자동으로 갱신되지 않습니다. 코드가 크게 바뀌었거나 검색 결과가 오래된 것 같다면 다시 인덱싱하세요.

`--content docs`로 문서와 산문을, `--content config`로 yaml/toml 등 설정 파일을, `--content all`로 모두 검색할 수 있습니다.

```bash
csp search "deployment guide" ./my-project --content docs
csp search "database host port" ./my-project --content config
csp search "authentication" ./my-project --content all
```

`csp find-related`로 기존 위치와 비슷한 코드를 찾을 수 있습니다 (이전 검색 결과의 `file_path`와 `line`을 사용).

```bash
csp find-related src/auth.ts 42 ./my-project
```

`search`와 마찬가지로 `find-related`도 `--index` 인자를 받습니다.

`path`를 생략하면 현재 디렉터리를 사용합니다. git URL도 받습니다.

`csp`가 `$PATH`에 없다면 `bunx @pleaseai/csp`로 대체하세요.

### Workflow

1. `csp index -o cached_index`로 리포지토리를 인덱싱.
2. `csp search`로 관련 청크를 찾기. 인덱스를 넘기면 더 빠릅니다.
3. 문서는 `--content docs`, 설정 파일은 `--content config`, 전부는 `--content all`.
4. 반환된 청크로 컨텍스트가 부족할 때만 파일을 전체 읽기.
5. 유망한 결과에서 `csp find-related`로 비슷한 구현을 추가 탐색.
6. 정확한 리터럴 매칭이나 문자열 확인이 필요할 때만 grep 사용.
````

</details>

### 서브 에이전트

서브 에이전트를 지원하는 하니스에서는 `csp-search` 전용 서브 에이전트를 설치해 검색이 별도 컨텍스트에서 동작하도록 할 수 있습니다 (CLI 필요).

```bash
csp init   # Claude Code → .claude/agents/csp-search.md
```

다른 하니스(Cursor, Codex, OpenCode 등)는 아래 [서브 에이전트 설정](#서브-에이전트-설정) 섹션을 참고하세요.

<details>
<summary>csp 업데이트</summary>

```bash
bun update -g @pleaseai/csp     # bun
npm update -g @pleaseai/csp     # npm
pnpm update -g @pleaseai/csp    # pnpm
```

</details>

## 주요 기능

- **빠름**: 평균적인 리포지토리를 1초 이내에 인덱싱하고, 쿼리는 밀리초 단위로 응답합니다. 전부 CPU에서 동작합니다.
- **정확함**: 하이브리드 검색(밀집 임베딩 + BM25)과 코드 인지형 리랭킹을 결합합니다.
- **토큰 효율적**: 관련 청크만 반환하므로 grep+read 대비 약 98% 적은 토큰을 사용합니다.
- **제로 셋업**: API 키, GPU, 외부 서비스 없이 CPU에서 동작합니다.
- **MCP 서버**: Claude Code, Cursor, Codex, OpenCode, VS Code 등 MCP 호환 에이전트와 함께 사용 가능합니다.
- **로컬 / 원격**: 로컬 경로 또는 git URL을 모두 받습니다.
- **단일 바이너리**: 자체 완결형 [Rust](https://www.rust-lang.org/) 실행 파일 — Homebrew로는 런타임 없이 설치하거나, npm/bun/pnpm으로 설치(`PATH`에 Node 22+ 또는 Bun 필요).

## MCP 서버

`csp`는 MCP 서버로 동작할 수 있어 에이전트가 어떤 코드베이스든 직접 검색할 수 있습니다. 리포지토리는 필요할 때 클론되어 인덱싱됩니다. 서버는 세션 동안 인메모리 핫 캐시를 유지하며, CLI와 동일한 디스크 캐시 `~/.csp/index/`를 공유하므로 한 번 만든 인덱스는 양쪽에서 재사용됩니다. 로컬 경로는 파일 변경을 감시해 자동으로 재인덱싱하며, 디스크 캐시 재사용은 소스 콘텐츠 해시로 무효화됩니다.

### 설정

<details>
<summary>Claude Code</summary>

```bash
claude mcp add csp -s user -- bunx @pleaseai/csp mcp
```

</details>

<details>
<summary>Cursor</summary>

`~/.cursor/mcp.json` (또는 프로젝트 내 `.cursor/mcp.json`)에 추가:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Codex</summary>

`~/.codex/config.toml`에 추가:

```toml
[mcp_servers.csp]
command = "bunx"
args = [
  "@pleaseai/csp",
  "mcp"
]
```

</details>

<details>
<summary>OpenCode</summary>

`~/.opencode/config.json`에 추가:

```json
{
  "mcp": {
    "csp": {
      "type": "local",
      "command": ["bunx", "@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>VS Code</summary>

프로젝트의 `.vscode/mcp.json` (또는 사용자 프로필의 `mcp.json`)에 추가:

```json
{
  "servers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>GitHub Copilot CLI</summary>

`~/.copilot/mcp-config.json`에 추가:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Windsurf</summary>

`~/.codeium/windsurf/mcp_config.json`에 추가:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Gemini CLI</summary>

`~/.gemini/settings.json`에 추가:

```json
{
  "mcpServers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

<details>
<summary>Zed</summary>

`~/.config/zed/settings.json` (또는 프로젝트 내 `.zed/settings.json`)에 추가:

```json
{
  "context_servers": {
    "csp": {
      "command": "bunx",
      "args": ["@pleaseai/csp", "mcp"]
    }
  }
}
```

</details>

### 도구

| 도구 | 설명 |
|------|------|
| `search` | 자연어 또는 코드 쿼리로 코드베이스를 검색. `repo`는 로컬 디렉터리 경로 또는 https:// git URL. |
| `find_related` | 파일 경로와 라인 번호를 받아, 해당 위치의 코드와 의미적으로 유사한 청크를 반환. |

기본적으로 MCP 서버는 코드 파일만 인덱싱합니다. 문서/설정/전체를 함께 인덱싱하려면 명령에 `--content docs`, `--content config`, `--content all` 또는 조합(예: `--content code docs`)을 추가하세요. 예를 들어 Claude Code에서는 `claude mcp add csp -s user -- bunx @pleaseai/csp mcp --content all`.

## 서브 에이전트 설정

Claude Code, Gemini CLI, Cursor, OpenCode, GitHub Copilot CLI, Kiro, Antigravity, Command Code, Pi, Reasonix는 모두 전용 `csp` 검색 서브 에이전트를 지원합니다. 프로젝트 루트에서 `csp init`을 한 번 실행하세요.

```bash
csp init                      # Claude Code  → .claude/agents/csp-search.md
csp init --agent gemini       # Gemini CLI   → .gemini/agents/csp-search.md
csp init --agent cursor       # Cursor       → .cursor/agents/csp-search.md
csp init --agent opencode     # OpenCode     → .opencode/agents/csp-search.md
csp init --agent copilot      # Copilot CLI  → .github/agents/csp-search.md
csp init --agent kiro         # Kiro         → .kiro/agents/csp-search.md
csp init --agent antigravity  # Antigravity  → .antigravity/agents/csp-search.md
csp init --agent commandcode  # Command Code → .commandcode/agents/csp-search.md
csp init --agent pi           # Pi           → .pi/agents/csp-search.md
csp init --agent reasonix     # Reasonix     → .reasonix/agents/csp-search.md
```

`csp`가 `$PATH`에 없다면 명령 앞에 `bunx @pleaseai/csp`를 붙이세요.

## CLI

`csp`는 독립 실행형 CLI로도 제공됩니다. 스크립트나 MCP 세션 없이 검색 결과를 받고 싶을 때 유용합니다.

```bash
# 로컬 리포지토리 검색
csp search "authentication flow" ./my-project

# 반복 검색을 빠르게 하려면 먼저 인덱싱 (--index는 아래 모든 명령과 호환)
csp index ./my-project -o my-index
csp search "authentication flow" --index my-index

# 원격 리포지토리 검색 (필요할 때 클론)
csp search "save model to disk" https://github.com/MinishLab/model2vec

# 결과 개수 제한
csp search "save model to disk" ./my-project --top-k 10

# 코드 대신 docs/config/전체 검색
csp search "deployment guide" ./my-project --content docs   # 또는 config, all

# 특정 위치와 비슷한 코드 찾기
csp find-related src/auth.ts 42 ./my-project
```

`--content`는 `code` (기본), `docs`, `config`, `all`을 받습니다. `path`를 생략하면 현재 디렉터리를 사용합니다. git URL도 받습니다. `csp`가 `$PATH`에 없다면 `bunx @pleaseai/csp`로 대체하세요.

`csp search`나 `csp find-related`를 `--index` 없이 실행하면, `csp`는 소스와 콘텐츠 선택을 키로 하여 글로벌 캐시 `~/.csp/index/`에 자동으로 인덱싱·캐시합니다. 다음 실행 때 캐시를 재사용하며, 소스 파일이 바뀌면 콘텐츠 해시로 자동 무효화되므로 수동으로 다시 인덱싱할 필요가 없습니다. `--index <경로>`를 지정하면 그 경로를 그대로 사용하고 자동 캐시를 우회합니다. `csp index -o <경로>`는 명시적 영속화 전용(`-o` 필수)이며 자동 캐시와는 독립적입니다.

<details>
<summary>토큰 절약량 보기</summary>

`csp savings`는 모든 검색에서 `csp`가 절약한 토큰량을 보여줍니다.

```bash
csp savings           # 기간별 요약
csp savings --verbose # 호출 유형별 분해 포함
```

```
  Csp Token Savings
  ════════════════════════════════════════════════════════════════════════

  Total saved:  ~1.2M tokens  (89%)
  Total calls:  1.4k
  Efficiency:  █████████████████████░░░  89%

  By Period
  ────────────────────────────────────────────────────────────────────────
  Period             Calls           Saved  Ratio
  ────────────────────────────────────────────────────────────────────────
  Today                 42    ~58.4k tokens  ███████████████████████░  95%
  Last 7 days          287   ~312.4k tokens  █████████████████████░░░  90%
  All time             1.4k     ~1.2M tokens  █████████████████████░░░  89%
```

절약량 계산: 각 호출마다 반환된 청크가 속한 파일들의 총 문자 수와 반환된 스니펫의 문자 수를 기록합니다. 절약된 토큰 추정치는 `(파일 문자 수 − 스니펫 문자 수) / 4` (1토큰 ≈ 4문자). 이는 보수적인 추정으로, 기준선은 "에이전트가 매칭된 파일을 통째로 읽는다"는 일반적인 코딩 에이전트 동작입니다.

stdout이 컬러를 지원하는 TTY일 때 출력에 색이 입혀집니다(`NO_COLOR`, `dumb` 터미널, 파이프 연결 시에는 비활성화). `--verbose`를 주면 "By Call Type" 분해가 추가됩니다.

통계는 `~/.csp/savings.jsonl`에 저장됩니다.

</details>

<details>
<summary>캐시 비우기</summary>

`csp clear`는 캐시된 데이터를 삭제합니다.

```bash
csp clear savings  # ~/.csp/savings.jsonl 삭제
csp clear index    # 글로벌 인덱스 캐시 ~/.csp/index/ 삭제
csp clear all      # 인덱스 캐시와 savings 모두 삭제
```

`clear index`는 글로벌 인덱스 캐시 `~/.csp/index/`(여기에 `csp search`/`find-related`가 인덱스를 자동 캐시합니다)를 삭제하고 제거된 캐시 엔트리 수를 보고합니다. `~/.csp/savings.jsonl`은 보존됩니다. `clear all`은 `~/.csp/index/`와 `~/.csp/savings.jsonl`을 각각 독립적으로 삭제합니다.

`csp index -o <경로>`로 명시적으로 기록한 인덱스 경로는 자동 캐시 대상이 아니므로 `clear`가 건드리지 않습니다. 해당 디렉터리는 직접 삭제하세요.

</details>

<details>
<summary>라이브러리 사용</summary>

`csp`는 Rust 라이브러리 크레이트로도 사용할 수 있습니다. 짧은 이름 `csp`가 이미 선점되어 있어 crates.io에는 [**`code-search-please`**](https://crates.io/crates/code-search-please)로 배포됩니다. 라이브러리 이름은 `csp` 그대로이므로 의존성은 `code-search-please`로 추가하되 코드에서는 `use csp::...`를 씁니다. `CspIndex`(`from_path` / `from_git` / `search` / `find_related`)와 `ContentType` enum, 랭킹 파이프라인을 노출합니다.

```toml
[dependencies]
code-search-please = "0.1"
```

```rust
use std::path::Path;
use csp::indexing::index::{CspIndex, LoadOptions, QueryOptions};

// 로컬 디렉터리를 인덱싱하고 검색
let index = CspIndex::from_path(Path::new("./my-project"), &LoadOptions::default())?;
let results = index.search("save model to disk", &QueryOptions { top_k: Some(3), ..Default::default() });

for r in &results {
    println!("{}:{}-{}", r.chunk.file_path, r.chunk.start_line, r.chunk.end_line);
}
```

> 크레이트는 crates.io에 [`code-search-please`](https://crates.io/crates/code-search-please)로 배포되며 라이브러리 이름은 `csp`입니다. (npm 패키지는 `csp` 바이너리를 런처 뒤에 담아 배포할 뿐, JavaScript API를 노출하지 않습니다.)

</details>

## 동작 원리

`csp`는 [tree-sitter](https://tree-sitter.github.io/)로 각 파일을 코드 인지형 청크로 분할한 뒤, 두 개의 상호 보완적인 검색기로 쿼리를 모든 청크와 점수화합니다. 의미 유사도를 위한 정적 [Model2Vec](https://github.com/MinishLab/model2vec) 임베딩(코드 특화 `potion-code-16M` 모델 사용)과, 식별자/API명 등 어휘 매칭을 위한 BM25입니다. 두 점수 리스트는 Reciprocal Rank Fusion(RRF)으로 결합됩니다.

결합 후에는 코드 인지형 신호들로 결과를 재정렬합니다.

<details>
<summary><b>랭킹 신호</b></summary>

- **적응형 가중치.** `Foo::bar`, `_private`, `getUserById` 같은 심볼형 쿼리는 어휘 가중치를 더 받고, 자연어 쿼리는 의미/어휘 검색기 사이에서 균형을 유지합니다.
- **정의 부스트.** 쿼리된 심볼을 *정의*하는 청크(`class`, `function`, `interface` 등)는 단순 *참조*하는 청크보다 상위에 위치합니다.
- **식별자 스템.** 쿼리 토큰을 스템 처리해 청크 내 식별자 스템과 매칭하고, 매칭된 청크에 추가 가중치를 부여합니다. 예: `parse config` 쿼리는 `parseConfig`, `ConfigParser`, `config_parser`가 포함된 청크를 부스트합니다.
- **파일 일관성.** 같은 파일의 여러 청크가 쿼리에 매칭되면 해당 파일을 부스트해, 단일 청크가 아닌 파일 수준의 관련성이 상위에 반영되도록 합니다.
- **노이즈 페널티.** 테스트 파일, `compat/`/`legacy/` 셰임, 예제 코드, `.d.ts` 선언 스텁은 순위가 낮아져 정규 구현이 먼저 노출됩니다.

</details>

임베딩 모델이 정적이라 쿼리 시점에 트랜스포머 forward pass가 없으므로, 위 모든 과정이 CPU에서 밀리초 단위로 동작합니다.

## 개발

라이브러리와 `csp` 바이너리는 Cargo 워크스페이스입니다(`crates/csp`, `crates/csp-cli`):

```bash
cargo build --release          # csp 바이너리 빌드
cargo test --workspace         # 테스트 실행
cargo fmt --all                # 포맷
cargo clippy --all-targets --all-features -- -D warnings   # 린트
```

## 크레딧

`csp`는 [MinishLab](https://github.com/MinishLab)의 [Thomas van Dongen](https://github.com/Pringled)과 [Stéphan Tulkens](https://github.com/stephantul)가 만든 [Semble](https://github.com/MinishLab/semble)의 Rust 포트입니다. 알고리즘, 랭킹 신호, 전체 아키텍처의 공로는 모두 원저자에게 있으며, 본 프로젝트는 이를 Rust로 이식한 것입니다.

연구에서 핵심 아이디어를 인용하시려면 원본 Semble 논문을 인용해 주세요.

```bibtex
@software{minishlab2026semble,
  author       = {{van Dongen}, Thomas and Stephan Tulkens},
  title        = {Semble: Fast and Accurate Code Search for Agents},
  year         = {2026},
  publisher    = {Zenodo},
  doi          = {10.5281/zenodo.19785932},
  url          = {https://github.com/MinishLab/semble},
  license      = {MIT}
}
```

## 라이선스

[MIT](./LICENSE) © [이민수](https://github.com/amondnet)

`csp`는 동일하게 MIT 라이선스인 [MinishLab/semble](https://github.com/MinishLab/semble)의 파생 저작물입니다.
