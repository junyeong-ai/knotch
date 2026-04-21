# Knotch

[![Rust](https://img.shields.io/badge/rust-1.94.0-orange?logo=rust&style=flat-square)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2024-informational?style=flat-square)](https://doc.rust-lang.org/edition-guide/)
[![Unsafe](https://img.shields.io/badge/unsafe-forbidden-success?style=flat-square)](#품질-게이트)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-lightgrey?style=flat-square)](#라이선스)

> **[English](README.md)** | **한국어**

> **Git 상관 이벤트 소싱 워크플로우 상태 — AI 에이전트를 위해 설계됨.**

Knotch 는 Rust 라이브러리이자 `knotch` CLI 입니다. AI 에이전트에게
워크플로우 상태를 관리할 단일하고 감사 가능한 표면을 제공합니다.
에이전트가 수행하는 모든 행동은 불변 이벤트로 기록되고, 모든 읽기는
그 이벤트 로그에 대한 순수 projection 입니다. 로그를 쓰는 경로는
하나뿐이며, 커널은 I/O 를 전혀 수행하지 않기 때문에 replay 후에도
불변 조건이 유지됩니다.

---

## 왜 knotch 인가

AI 에이전트는 실제 워크플로우를 구동합니다. 단일 기능 단위 하나가
보통 도구 호출, git 커밋, 페이즈 완료, 서브에이전트 위임, 모델
전환, 사람의 승인까지 엄청난 양의 상태 변경을 만들어냅니다. 순진한
상태 저장소 (`state.json`, `status.md`, DB row) 는 이 변경들을
이전 진실을 덮어쓰며 흡수합니다. 여기서 세 가지 실패 모드가 직접
발생하고 — knotch 의 설계는 각각에 대한 정면 대응입니다.

### 1. 재시도가 로그를 오염시키거나 업데이트를 잃어버림

에이전트는 재시도합니다. at-least-once 전달은 선택이 아니라 현실
입니다. `state.json` 에서는 재시도된 쓰기가 row 를 중복 생성하거나
첫 번째를 조용히 덮어쓰게 됩니다. Knotch 는 모든 proposal 에
content-addressed `Fingerprint` — dedup 튜플 `{workflow, body,
supersedes}` 의 RFC 8785 JCS canonical 형식 위의 BLAKE3 해시 — 를
붙입니다. 재시도는 `AppendReport::rejected` 에 `"duplicate"` 이유와
함께 조용히 안착하는 no-op 이며, 이것은 에러가 아니라
**at-least-once 전달의 성공 신호** 입니다.

근거: `crates/knotch-kernel/src/fingerprint.rs::fingerprint_proposal`,
[`.claude/rules/fingerprint.md`](.claude/rules/fingerprint.md).

### 2. Causation 이 첫 번째 롤업에서 붕괴됨

"누가, 언제, 어느 세션에서, 어떤 모델로 결정했는가?" 는 모든 감사가
결국 묻는 질문입니다. `state.json` 은 세션·서브에이전트·도구
호출·모델을 남아있는 필드 안에 뭉뚱그립니다. Knotch 는 모든
이벤트에 타입화된 `Causation` 을 찍습니다:

```
Causation { source, session, agent_id, trigger }
```

- `source` 는 `Cli | Hook | Observer` — 세 개의 unit variant,
  프로젝트 브랜드 확장 없음.
- `trigger` 는 `Command { name } | GitHook { name } |
  ToolInvocation { tool, call_id } | Observer { name }` — 전부
  struct-form 이므로 필드 추가가 fingerprint-stable.
- `agent_id` 는 최상위 `Option<AgentId>` 필드 — `session` 에서
  합성되지 않으므로 "이 서브에이전트가 무엇을 했는가?" 는 단일
  필드 필터.
- 모델 identity 는 의도적으로 causation 에 없습니다; 대신 전용
  `ModelSwitched` 이벤트에 기록되어 세션 중간의 `/model` 전환이
  stale-copy 관리 없이 정확히 기록됩니다.

근거: `crates/knotch-kernel/src/causation.rs`,
[`.claude/rules/causation.md`](.claude/rules/causation.md).

### 3. proposal 간 원자적 불변 조건이 없음

"페이즈를 완료 처리하고, 게이트 결정을 기록하고, `in_review` 로
전환" 은 하나의 논리적 결정입니다. `state.json` 파일들은 이 셋을
조율할 방법이 없습니다. Knotch 의 `Repository::append` 는 unit 별
lock 을 쥔 채, 방금 로드한 로그에 대해 15개의 variant 별
precondition 을 실행하고, `AppendMode::AllOrNothing` 모드에서
proposal 하나라도 rejection 되면 전체 배치를 롤백합니다.

근거: `crates/knotch-storage/src/file_repository.rs::commit_batch`,
[`.claude/rules/append-flow.md`](.claude/rules/append-flow.md),
[`.claude/rules/preconditions.md`](.claude/rules/preconditions.md).

---

## 빠른 시작

```bash
# 프로젝트에 워크스페이스 스캐폴딩 (설치는 아래 §설치 참고)
knotch init --with-hooks

# unit 생성 후 phase 기록
knotch unit init feat-auth
knotch mark completed specify
knotch gate g0-scope pass "scope bounded to OAuth2 password grant"

# 상태 조회
knotch show feat-auth               # projection 요약
knotch show feat-auth --format json # 기계 판독용
knotch log feat-auth                # 원본 이벤트 스트림
```

모든 명령어는 `--json` (기계 출력) 과 `--quiet` 전역 플래그를
지원하며, 각 하위 커맨드가 두 플래그를 균일하게 처리합니다.

---

## 동작 원리

### 스냅샷이 아닌 이벤트

대부분의 워크플로우 도구는 unit 의 *현재 상태* 를 하나의 파일
(`state.json`, 데이터베이스 행, YAML 블록) 에 저장합니다. 매번
write 가 이전 진실을 덮어씁니다. 여기서 세 가지 실패 모드가
발생합니다:

| 스냅샷 모델 | 이벤트 소싱 모델 (knotch) |
|---|---|
| 동시 writer 두 개가 경쟁하며 업데이트를 잃음 | 라인 카운트에 대한 optimistic CAS — 두 번째 writer 는 최신 로그에 대해 재시도 |
| 상태 롤백에 감사 흔적이 남지 않음 | `EventSuperseded` 가 불변 롤백 이벤트로 방출됨 |
| "누가 이것을 결정했는가?" 가 복구 불가 | 모든 이벤트는 `Causation` 을 담음 — `source`, `session`, `agent_id`, `trigger` |
| 파일 간 silent divergence (status.md vs status.json) | Markdown frontmatter 는 원장의 projection — `knotch-frontmatter` 가 동기화 |

### Append 경로

`Repository::append` 를 통한 모든 write 는 unit 의 락을 쥔 채 아래
순서대로 수행됩니다. 이 순서는 어댑터가 강제하며
[`.claude/rules/append-flow.md`](.claude/rules/append-flow.md) 에
검증됩니다.

```mermaid
flowchart LR
    A["Proposal<W>"] --> L["1. Acquire lock<br/><i>fs4 + tokio Mutex</i>"]
    L --> S["2. Load snapshot<br/><i>parse header + events</i>"]
    S --> F["3. Compute existing<br/>fingerprints"]
    F --> D{"4a. Dedup?"}
    D -- "duplicate" --> RJ["rejected[]<br/><i>success signal</i>"]
    D -- "new" --> P{"4b. Precondition?"}
    P -- "fail" --> RJ
    P -- "ok" --> M["4c-d. Extension +<br/>monotonic 'at' check"]
    M --> ST["4e. Stamp<br/><i>EventId::new_v7,<br/>Timestamp::now</i>"]
    ST --> C["6. Commit<br/><i>optimistic CAS<br/>on line count</i>"]
    C --> B["7. Broadcast<br/><i>per-unit<br/>subscribers</i>"]
```

1. **락 획득** — `FileLock` (프로세스 간: `fs4` advisory lock +
   `rustix` stale-lease 감지) + 프로세스 내: 단위별
   `tokio::sync::Mutex`.
2. **스냅샷 로드** — JSONL 로그 헤더 + 이벤트 파싱.
3. **기존 fingerprint 계산** — dedup 을 위함.
4. **각 proposal 마다** (순서가 중요):
   - **Dedup 먼저** — fingerprint 가 이미 존재하면 `rejected` 에
     `"duplicate"` 이유와 함께 추가.
   - **Precondition** — `EventBody::check_precondition` 이
     variant 별로 디스패치 (자세히:
     [`.claude/rules/preconditions.md`](.claude/rules/preconditions.md)).
   - **Extension precondition** — `ExtensionKind::check_extension`.
   - **Monotonic `at`** — `stamp_monotonic(&clock, last_at)` 을
     통해 `at` 가 항상 마지막 이벤트보다 엄격히 크게 찍힘. NTP
     보정·VM suspend 등 벽시계 역행에도 자가복구.
   - **Stamp + working log 에 추가** — 다음 proposal 이 그 상태를
     보고 검증할 수 있도록.
5. **All-or-nothing rollback** — `AppendMode::AllOrNothing` 에서
   하나라도 rejection 이 있으면 전체 배치 폐기.
6. **Commit** via `Storage::append(unit, expected_len, lines)`.
   `expected_len` 에 대한 optimistic CAS; `LogMutated` 는 최대 5회
   bounded exponential backoff (25ms 베이스, 총 ≤775ms) 로 재시도.
7. **Broadcast** 를 unit 별 subscriber 에게. receiver 없으면 무시.

`knotch-linter` 룰 **R1** (`DirectLogWriteRule`) 은 `knotch-storage`
외부의 어떤 crate 도 `log.jsonl` 에 직접 write 하는 것을 정적으로
금지합니다. 리뷰가 아닌 **빌드 실패**로 막습니다.

### Writer 는 4개의 tier 로 구분

15개의 `EventBody` variant 가 4개의 tier 에 나뉘며, 모든 variant 는
단 하나의 canonical emitter 를 가집니다.
[`.claude/rules/event-ownership.md`](.claude/rules/event-ownership.md)
의 표가 authoritative — `cargo xtask docs-lint` 가 분할이 코드와
일치하는지 검증합니다.

```mermaid
flowchart LR
    subgraph writers["4 writer tiers"]
        H["Hook<br/><i>deterministic<br/>auto-record</i>"]
        S["Skill<br/><i>agent judgment</i>"]
        C["CLI<br/><i>human / scripted</i>"]
        R["Reconciler<br/><i>external-state<br/>observation</i>"]
    end
    H -->|MilestoneShipped<br/>MilestoneReverted<br/>ModelSwitched<br/>ToolCallFailed<br/>SubagentCompleted| L[("log.jsonl")]
    S -->|PhaseCompleted<br/>PhaseSkipped<br/>GateRecorded<br/>StatusTransitioned| L
    C -->|UnitCreated<br/>EventSuperseded<br/>ApprovalRecorded| L
    R -->|MilestoneVerified<br/>ReconcileFailed<br/>ReconcileRecovered| L
```

| Tier | 트리거 | 소유 variant |
|---|---|---|
| **Hook** | Claude Code hook 이벤트 | `MilestoneShipped`, `MilestoneReverted`, `ModelSwitched`, `ToolCallFailed`, `SubagentCompleted` |
| **Skill** | 에이전트가 `/knotch-*` 호출 | `PhaseCompleted`, `PhaseSkipped`, `GateRecorded`, `StatusTransitioned` |
| **CLI** | 사람이 `knotch <cmd>` 실행 | `UnitCreated`, `EventSuperseded`, `ApprovalRecorded` |
| **Reconciler** | observer 가 외부 상태 관찰 | `MilestoneVerified`, `ReconcileFailed`, `ReconcileRecovered` |

다른 writer 는 없습니다. Tier 분리는 기계적으로 강제됩니다: hook 과
skill 은 `knotch-agent` 헬퍼를 경유하고, CLI 는 동일 헬퍼에
바인딩되며, `Repository::append` 가 네 tier 를 모두 거치는 유일한
진입점입니다.

### 읽기는 projection

모든 read API 는 로그 위의 순수 함수입니다. 8개의 public projection
이 `crates/knotch-kernel/src/project.rs` 에 있습니다:

| 함수 | 반환 |
|---|---|
| `current_phase(&W, &log)` | completed/skipped 되지 않은 첫 required phase |
| `last_completed_phase(&log)` | 가장 최근 `PhaseCompleted` |
| `current_status(&log)` | 마지막 `StatusTransitioned` target |
| `shipped_milestones(&log)` | `MilestoneShipped` 이벤트가 있는 milestone |
| `model_timeline(&log)` | 시간순 `(timestamp, model)` 전환 |
| `subagents(&log)` | supersede-aware 서브에이전트 명단 |
| `tool_call_timeline(&log, …)` | attempt 카운터 포함 도구별 실패 이력 |
| `effective_events(&log)` | superseded 를 제외한 로그 replay |

`knotch-query` crate 는 AND-조합 predicate 의 cross-unit
`QueryBuilder<W>` 를 노출하며, `knotch-tracing` 은 append 당
구조화된 span 을 방출합니다. 지속 업데이트를 구독할 에이전트는
`Repository::subscribe(unit) -> impl Stream<Item = SubscribeEvent<W>>`
를 사용합니다.

### 아키텍처

knotch 는 hexagonal ports-and-adapters 패턴을 따릅니다.
`knotch-kernel` 은 pure 합니다 — `knotch-linter` 룰 **R3**
(`KernelNoIoRule`) 이 kernel + proto crate 에서 `std::fs`, `std::net`,
`tokio::fs`, `tokio::net`, `gix` 임포트를 금지합니다. 어댑터는 포트를
구현하고, composition crate 가 이들을 조립합니다.

```mermaid
graph TB
    subgraph agent["Agent runtime (Claude Code / third-party)"]
        TC["tool call"]
    end

    subgraph kernel["knotch-kernel · knotch-proto (pure, no I/O)"]
        EV["Event&lt;W&gt; envelope<br/>+ Causation"]
        PR["check_precondition"]
        FP["fingerprint_proposal<br/>BLAKE3(salt ‖ JCS)"]
        PJ["project::*"]
    end

    subgraph adapters["Adapters (concrete I/O)"]
        REPO["FileRepository&lt;W&gt;"]
        STORE["FileSystemStorage<br/>atomic rename"]
        LOCK["FileLock<br/>fs4 + rustix"]
        VCS["GixVcs<br/>Verify / Pending / Missing"]
    end

    subgraph compose["Composition"]
        OBS["Observers<br/>git · artifact ·<br/>pending · subprocess"]
        REC["Reconciler<br/>deterministic merge"]
        QRY["QueryBuilder"]
    end

    TC -->|"Proposal&lt;W&gt;"| PR
    PR --> FP
    FP --> REPO
    REPO --> STORE
    REPO --> LOCK
    OBS --> VCS
    OBS --> REC
    REC -->|"batch append"| REPO
    REPO --> PJ
    PJ --> QRY
```

**하나의 제네릭 파라미터가 모든 API 에 관통**합니다: `W: WorkflowKind`
(`Phase`, `Milestone`, `Gate`, `Extension` associated type 을 담음).
별도 네 개의 bound 가 아닌 — RFC 0002 "single bound" 설계입니다.

### 워크스페이스 레이아웃

```
knotch/
├── crates/
│   ├── knotch-kernel/        # Pure: Event<W>, Repository, precondition, projection
│   ├── knotch-proto/         # Pure: 와이어 포맷, JCS, schema versioning
│   ├── knotch-derive/        # WorkflowKind 보일러플레이트용 proc-macro
│   ├── knotch-storage/       # 어댑터: JSONL FileSystemStorage + FileRepository
│   ├── knotch-lock/          # 어댑터: 프로세스 간 FileLock (fs4 + rustix)
│   ├── knotch-vcs/           # 어댑터: GixVcs (pure-Rust git, C 의존성 없음)
│   ├── knotch-workflow/      # 정식 Knotch 워크플로우 + ConfigWorkflow 런타임
│   ├── knotch-schema/        # Tier-5: FrontmatterSchema + LifecycleFsm
│   ├── knotch-frontmatter/   # Tier-5: Markdown ↔ 원장 status 동기화
│   ├── knotch-adr/           # Tier-5: ADR lifecycle WorkflowKind
│   ├── knotch-observer/      # Observer trait + git/artifact/pending/subprocess
│   ├── knotch-reconciler/    # 결정론적 observer composition
│   ├── knotch-query/         # Cross-unit QueryBuilder (AND-composed predicate)
│   ├── knotch-tracing/       # 안정적 tracing attribute schema + span helper
│   ├── knotch-linter/        # cargo knotch-linter (R1/R2/R3 enforcement)
│   ├── knotch-agent/         # Claude Code hook/skill 통합 라이브러리
│   ├── knotch-cli/           # `knotch` 레퍼런스 바이너리
│   └── knotch-testing/       # 개발용: InMemoryRepository + simulation harness
├── examples/                 # minimal, pr-workflow, compliance, artifact-probes,
│                             # batch-append, interactive-observer, subprocess-observer-{node,py},
│                             # workflow-{spec-driven,vibe}-case-study
├── plugins/knotch/           # Claude Code 플러그인 번들 (hooks/ + skills/)
├── .claude/rules/            # 구조적 불변 조건 (Claude Code 가 path-scope 로 로드)
├── .claude/skills/           # 에이전트 스킬 (knotch-{mark,gate,query,transition,approve})
├── docs/public_api/          # Per-crate public-API baseline (CI 에서 diff)
├── docs/migrations/          # 어답터 migration playbook
└── xtask/                    # cargo xtask {ci,docs-lint,public-api,plugin-sync}
```

각 `crates/<name>/CLAUDE.md` 가 그 crate 의 역할 + 확장 레시피를
담습니다. 크레이트별 구조 룰은 `.claude/rules/` 에서 `@`-import
되므로 Claude 는 현재 다루는 파일과 관련된 내용만 로드합니다.

---

## 연동 가치

knotch 를 연결한 뒤 AI 에이전트 어답터에게 달라지는 것이 바로 이
섹션입니다. 관통하는 패턴: **hook 이 결정론적 이벤트를 자동
기록하고, skill 이 에이전트의 판단을 명시적으로 기록합니다.**

### Hook 자동 기록

Claude Code 는 다섯 종류의 hook 이벤트를 노출합니다.
[`plugins/knotch/hooks/hooks.json`](plugins/knotch/hooks/hooks.json)
번들이 각각을 `knotch hook` 서브커맨드로 라우팅하여 적절한
`EventBody` 를 append 합니다 — 에이전트 개입 불필요.

```mermaid
flowchart LR
    subgraph cc["Claude Code events"]
        SS["SessionStart"]
        PRE["PreToolUse<br/><i>Bash</i>"]
        POST["PostToolUse<br/><i>Bash</i>"]
        FAIL["PostToolUseFailure"]
        SUB["SubagentStop"]
        SK["/knotch-* skill"]
    end

    subgraph dispatch["knotch hook / subcommand"]
        LC["load-context"]
        CC["check-commit<br/>guard-rewrite"]
        VC["verify-commit<br/>record-revert"]
        RTF["record-tool-failure"]
        RS["record-subagent"]
        SKL["mark / gate /<br/>transition / approve"]
    end

    subgraph out["Events appended"]
        MS["ModelSwitched"]
        MSH["MilestoneShipped<br/>MilestoneReverted"]
        TCF["ToolCallFailed"]
        SC["SubagentCompleted"]
        PC["PhaseCompleted / Skipped<br/>GateRecorded<br/>StatusTransitioned<br/>ApprovalRecorded"]
    end

    SS --> LC --> MS
    PRE --> CC
    POST --> VC
    VC --> MSH
    FAIL --> RTF --> TCF
    SUB --> RS --> SC
    SK --> SKL --> PC
```

구체 커버리지 — hook 서브커맨드 별 한 bullet:

- **`load-context`** (SessionStart) 는 active-unit 컨텍스트를
  대화에 주입하고, payload 의 `model` 을
  `project::model_timeline(log).last()` 와 비교해 다르면
  `ModelSwitched` 이벤트를 append. 시작·resume·`/clear`·`/compact`
  등 Claude Code 가 `SessionStart` 를 재발화하는 모든 lifecycle
  지점을 커버. **Zero config.**
- **`check-commit`** (PreToolUse, `Bash(git commit *)`) 는
  `-m` / `--message=` / `-F <file>` 를 `shell-words` 로 파싱해
  커밋 실행 전 `Knotch-Milestone: <id>` 트레일러를 검증.
- **`guard-rewrite`** (PreToolUse, 파괴적 git) 는 `[guard]` 정책에
  따라 `push --force`, `reset --hard`, `branch -D`, `checkout --`,
  `clean -f`, `rebase -i` 를 경고 또는 차단.
  `push --force-with-lease` 는 항상 예외.
- **`verify-commit`** (PostToolUse, `Bash(git commit *)`) 는 VCS 에
  커밋 존재를 확인하고
  `MilestoneShipped { status: Pending | Verified }` 를 append.
  `PendingCommitObserver` 가 이후 커밋이 main 에 도달하면
  Pending → Verified 로 승격.
- **`record-revert`** (PostToolUse, `Bash(git revert *)`) 는
  되돌려진 milestone 을 식별하여 `MilestoneReverted` 를 append.
- **`record-tool-failure`** (PostToolUseFailure) 는 에러 메시지를
  1 KiB 로 캡, `is_interrupt == true` (사용자 의도적 Esc / Ctrl-C
  는 실패가 아님) 는 필터, `ToolCallFailed` 를 방출. precondition 의
  `(tool, call_id)` 단조성 룰이 out-of-order 재시도를 잡음.
- **`record-subagent`** (SubagentStop) 는 transcript 경로 + 마지막
  메시지를 기록하고 `SubagentCompleted` 를 방출.
- **`refresh-context`** (UserPromptSubmit) 는 active-unit 컨텍스트
  를 재주입; 이벤트는 발생하지 않음.
- **`finalize-session`** (SessionEnd) 는 잔여 queue 크기를 기록
  하고, resume 가 아닌 경우 per-session 포인터를 정리.

`PostToolUse` hook 은 도구가 성공한 *이후* 에 발화하므로 차단할 수
없습니다. Repository/I/O 에러 시 3× 재시도 (50ms / 200ms / 800ms)
후 `knotch_agent::queue::enqueue_raw` 로 queue 에 넣고, queue-full
시 `~/.knotch/orphan.log` 로 fallback. `SessionStart` 가 queue 를
자동 드레인 — 일시 장애는 다음 세션에서 자가 복구. 전체 계약:
[`.claude/rules/hook-integration.md`](.claude/rules/hook-integration.md).

### Skill 명시적 액션

5개의 skill 이 에이전트가 실제로 내리는 판단을 다룹니다:

| Skill | 방출 | 에이전트 판단 |
|---|---|---|
| `/knotch-mark` | `PhaseCompleted` / `PhaseSkipped` | "이 phase 는 ...때문에 완료 / 스킵" |
| `/knotch-gate` | `GateRecorded { decision, rationale }` | 체크포인트에서 pass / fail / needs-revision (최소 8자 rationale) |
| `/knotch-transition` | `StatusTransitioned { target, forced, rationale }` | shipped / archived / abandoned; forced 는 rationale 필수 |
| `/knotch-approve` | `ApprovalRecorded` | 사람이 이전 이벤트에 co-sign |
| `/knotch-query` | *(읽기 전용 projection)* | 다음 액션 기획 전 현재 상태 조회 |

각 skill 은 `knotch` 서브커맨드로 shell out 합니다; hook 계약이
에이전트 도구 호출이든, 사람의 `knotch` 호출이든, `knotch-agent` 를
임베드한 서드파티 하네스든 동일 동작을 보장합니다.

### Git milestone 상관 end-to-end

**Milestone 은 opt-in 입니다.** `Knotch-Milestone: <id>` git 트레일러
가 명시된 커밋만 `MilestoneShipped` 이벤트를 만듭니다 — 설명문에서
slug 를 추정하는 휴리스틱 기반 인플레이션 없음. `git commit -m
"feat: add SSO\n\nKnotch-Milestone: m-sso"` 의 전체 흐름:

1. **`check-commit`** (PreToolUse) 이 `extract_milestone_id` 로
   milestone id 를 추출, 이미 shipped 가 아닌지 확인, 중복이면
   차단.
2. **`verify-commit`** (PostToolUse) 이 VCS 에서 커밋을 확인하고
   `MilestoneShipped { status: Pending | Verified }` 를 append
   (커밋이 이미 main 에 있으면 Verified, 아니면 Pending).
3. **`PendingCommitObserver`** (reconciler) 가 다음 `knotch reconcile`
   패스에서 커밋이 main 에 도달하면 `Pending` → `Verified` 로
   승격.
4. **`record-revert`** (`git revert` 의 PostToolUse) 가 원래
   milestone 으로 거슬러 올라가 `MilestoneReverted` 를 방출.

### Observer 와 reconciliation

4종의 built-in observer 가 `knotch-observer` 에 있습니다:

- **`GitLogObserver`** 는 `Vcs::log_since` 를 순회하며 커밋된
  리비전의 트레일러에서 `MilestoneShipped` / `MilestoneReverted`
  를 방출.
- **`ArtifactObserver`** 는 선언된 phase artifact 를 파일 시스템
  에서 스캔하여 나타나면 `PhaseCompleted` 방출.
- **`PendingCommitObserver`** 는 커밋이 main 에 도달하면
  `MilestoneShipped { status: Pending }` → `MilestoneVerified`
  로 승격.
- **`SubprocessObserver`** 는 외부 바이너리로 shell out;
  stdin 은 JSON `ObserverContext`, stdout 은 `Vec<Proposal<W>>`.
  Rust 외 파이프라인 (배포, 메트릭, 컴플라이언스 스캐너) 이
  원장에 feed 할 수 있음.

`knotch-reconciler` 가 observer 를 결정론적으로 조립: 로그 스냅샷
로드 → `tokio::task::JoinSet` 에 per-observer timeout 으로 spawn →
`Vec<(observer_name, Proposal<W>)>` 수집 → `(observer_name,
kind_ordinal, kind_tag)` 로 정렬 → 단일 `Repository::append` 배치
호출. Observer 오류는 패스를 중단시키지 않고
`ReconcileReport::observer_errors` 로 드러남.

### Observability join 포인트

`knotch-tracing` 은 append 당 구조화된 span 을 안정적인 attribute
schema 와 함께 방출합니다. 대시보드는 이 이름에 고정되며,
`cargo public-api` 가 rename 을 stable-surface 변경으로 잡습니다.

| Attribute | 값 |
|---|---|
| `knotch.unit.id` | `UnitId` 문자열 |
| `knotch.event.id` | 방출된 이벤트의 UUIDv7 |
| `knotch.event.kind` | body-variant 태그 (`milestone_shipped`, `phase_completed`, …) |
| `knotch.source` | `cli` / `hook` / `observer` |
| `knotch.session.id` | 대화 / 실행 스코프 |
| `knotch.agent.id` | 서브에이전트 UUID (스코프 내일 때) |
| `knotch.status.forced` | forced transition 시 `true` |
| `knotch.observer.name` | observer 의 `Observer::name()` (reconciler span) |
| `knotch.reconcile.accepted` | 이번 패스에서 accepted proposal 수 |
| `knotch.reconcile.rejected` | 이번 패스에서 rejected proposal 수 |
| `knotch.repository.op` | `append` / `load` / `subscribe` / `list_units` / `with_cache` |
| `knotch.repository.outcome` | `accepted` / `rejected` |

외부 observability (OTel, Prometheus) 가 동일 id 에서 join — 별도
correlation 레이어 불필요.

---

## 설치

### 빠른 설치 (권장)

**macOS / Linux**
```bash
curl -fsSL https://raw.githubusercontent.com/junyeong-ai/knotch/main/scripts/install.sh | bash
```

**Windows (PowerShell 7+)**
```powershell
iwr -useb https://raw.githubusercontent.com/junyeong-ai/knotch/main/scripts/install.ps1 | iex
```

인스톨러는 플랫폼을 감지하고, prebuilt 바이너리 + Claude Code 플러그인
번들을 다운로드하며, SHA256 를 검증하고, `$HOME/.local/bin` (POSIX)
또는 `$USERPROFILE\.local\bin` (Windows) 에 설치합니다. 필요하다면
플러그인을 `~/.claude/plugins/knotch` 에 함께 설치합니다. 터미널에서는
완전 인터랙티브 모드로 동작하며, CI 용으로 `--yes` 를 지원합니다.

### 지원 플랫폼

| OS | 아키텍처 | Target triple |
|---|---|---|
| Linux | x86_64 | `x86_64-unknown-linux-musl` (정적) |
| Linux | arm64 | `aarch64-unknown-linux-musl` (정적) |
| macOS | Intel + Apple Silicon | `universal-apple-darwin` (fat binary, ad-hoc codesign) |
| Windows | x86_64 | `x86_64-pc-windows-msvc` |

### 인스톨러 플래그

```
--version VERSION              특정 버전 설치 (기본: latest)
--install-dir PATH             바이너리 위치 (기본: $HOME/.local/bin)
--plugin user|project|none     플러그인 설치 레벨 (기본: user)
--from-source                  다운로드 대신 소스 빌드
--force                        기존 설치를 프롬프트 없이 덮어씀
--yes, -y                      비대화형 (모든 기본값 수용)
--dry-run                      계획만 출력, 실행하지 않음
```

모든 플래그는 대응하는 `KNOTCH_*` 환경 변수 (`KNOTCH_VERSION`,
`KNOTCH_INSTALL_DIR`, `KNOTCH_PLUGIN_LEVEL`, `KNOTCH_FROM_SOURCE`,
`KNOTCH_FORCE`, `KNOTCH_YES`, `KNOTCH_DRY_RUN`) 를 가집니다. ANSI
색상을 끄려면 `NO_COLOR=1` 을 설정하세요. 우선 순위는 flag > env >
기본값 입니다.

### cargo-binstall

```bash
cargo binstall knotch-cli
```

`crates/knotch-cli/Cargo.toml` 의 `[package.metadata.binstall]` 에
의해 인스톨 스크립트가 받는 동일한 prebuilt 아카이브를 다운로드
합니다.

<details>
<summary><b>체크섬 검증이 포함된 수동 설치</b></summary>

```bash
# 최신 릴리스 태그를 조회 (또는 특정 v*.*.* 태그를 직접 지정).
VERSION=$(curl -fsSL https://api.github.com/repos/junyeong-ai/knotch/releases/latest \
  | grep -oE '"tag_name": *"v[^"]+"' | head -n1 | cut -d'"' -f4 | sed 's/^v//')
TARGET=x86_64-unknown-linux-musl
BASE="https://github.com/junyeong-ai/knotch/releases/download/v$VERSION"
curl -fLO "$BASE/knotch-v$VERSION-$TARGET.tar.gz"
curl -fLO "$BASE/knotch-v$VERSION-$TARGET.tar.gz.sha256"
shasum -a 256 -c "knotch-v$VERSION-$TARGET.tar.gz.sha256"
tar -xzf "knotch-v$VERSION-$TARGET.tar.gz"
install -m 755 knotch "$HOME/.local/bin/knotch"
```

모든 릴리스 아티팩트에는 SLSA build provenance
(`actions/attest-build-provenance`) 가 추가로 서명됩니다 —
`gh attestation verify <archive>.tar.gz --owner junyeong-ai` 로
검증할 수 있습니다.

</details>

<details>
<summary><b>소스에서 빌드</b></summary>

```bash
git clone https://github.com/junyeong-ai/knotch
cd knotch
./scripts/install.sh --from-source
# 또는: cargo install --path crates/knotch-cli --locked
```

</details>

### 제거

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/junyeong-ai/knotch/main/scripts/uninstall.sh | bash

# Windows
iwr -useb https://raw.githubusercontent.com/junyeong-ai/knotch/main/scripts/uninstall.ps1 | iex
```

---

## CLI

```bash
# 워크스페이스 lifecycle
knotch init [--with-hooks] [--demo]      # knotch.toml + state/ + .knotch/ 스캐폴딩
knotch doctor                            # 헬스 체크 (룰 파일, 환경 변수, observer)
knotch migrate                           # schema-version 감지
knotch completions <bash|zsh|fish|…>     # 쉘 completion

# Unit 관리
knotch unit init <id>                    # unit 디렉토리 생성
knotch unit use <id>                     # active unit 지정 (.knotch/active.toml)
knotch unit list                         # 알려진 unit 열거
knotch unit current                      # 현재 active unit slug 출력
knotch current                           # `unit current` 의 alias

# 읽기 / 재생
knotch show <unit> [--format summary|brief|raw|json]
knotch log <unit>                        # 원본 JSONL 이벤트 스트림
knotch reconcile [--prune] [--prune-older <HOURS>] [--queue-only] [--unit <id>]

# 쓰기 (사람 중심, 드문 작업)
knotch supersede <event-id> <rationale>
knotch approve <unit> <event-id>

# 쓰기 (스킬 중심 — /knotch-* 스킬이 이들로 shell out)
knotch mark <completed|skipped> <phase> [--artifact <path>]... [--reason <text>]
knotch gate <gate-id> <decision> <rationale>
knotch transition <target> [--forced --reason <text>]

# Claude Code hook 디스패치 (stdin JSON)
knotch hook <load-context|check-commit|verify-commit|record-revert|
             guard-rewrite|record-subagent|record-tool-failure|
             refresh-context|finalize-session>
```

`--json` 과 `--quiet` 는 **전역** 플래그이며, 모든 하위 커맨드가
동일하게 처리합니다.

---

## 설정

### `knotch.toml`

```toml
# unit 별 로그가 위치할 디렉토리 (knotch.toml 기준 상대 경로).
state_dir = "state"
schema_version = 1

# guard-rewrite 정책. history 를 재작성하는 git 커맨드를
# hook 이 어떻게 처리할지 제어.
[guard]
rewrite = "warn"   # warn | block | off

# 이 프로젝트가 따르는 lifecycle. 기본값은 정식 Knotch 워크플로우;
# 도메인에 맞게 자유롭게 수정 가능.
[workflow]
name = "knotch"
schema_version = 1
terminal_statuses = ["archived", "abandoned", "superseded", "deprecated"]
known_statuses = [
    "draft", "in_progress", "in_review", "shipped",
    "archived", "abandoned", "superseded", "deprecated",
]

[workflow.required_phases]
tiny = ["specify", "build", "ship"]
standard = ["specify", "plan", "build", "review", "ship"]

[[workflow.phases]]
id = "specify"
# ... plan / build / review / ship

# 옵션: subprocess observer — `knotch reconcile` 중 각 바이너리로
# shell out. stdin 은 JSON ObserverContext, stdout 은
# JSON Vec<Proposal<W>>.
[[observers]]
name = "artifact-scan"
binary = "./tools/artifact-scan.py"
```

### 환경 변수

| 변수 | 소비자 | 기본값 |
|---|---|---|
| `KNOTCH_ROOT` | 전역 `--root` 오버라이드 | cwd 에서 `knotch.toml` 까지 상위 탐색 |
| `KNOTCH_UNIT` | active-unit 해결 (hook 체인의 최우선) | `.knotch/sessions/<id>.toml` → `.knotch/active.toml` |

모델 attribution 은 zero-config 입니다: Claude Code 가 모든
`SessionStart` payload 에 현재 모델을 스탬프하고,
`load-context` hook 이 마지막 기록된 값과 다를 때 `ModelSwitched`
이벤트를 append 합니다. 쉘 / `.envrc` 세팅 불필요 — `knotch init
--with-hooks` 가 Claude Code 설정에 `SessionStart` hook 을
연결했는지만 확인하면 됩니다.

### Guard 정책

`knotch.toml` 의 `[guard]` 섹션은 history 를 재작성하는 git 커맨드를
`guard-rewrite` hook 이 어떻게 처리할지 제어합니다
(`push --force`, `reset --hard`, `branch -D`, `checkout --`,
`clean -f`, `rebase -i/--root`):

| 정책 | 동작 |
|---|---|
| `warn` (기본) | Claude 가 context 에 경고 받음; 명령은 실행됨 |
| `block` | Hook 이 exit 2; 명령 취소 |
| `off` | 무음 no-op — 개인 실험 / 버려질 브랜치용 |

`git push --force-with-lease` 는 항상 예외 — Git 의 안전한 atomic-CAS
push 이기 때문입니다.

---

## Scope 계약

knotch 는 원장의 구조적 primitive 만 제공합니다. Scope 계약은
[`.claude/rules/governance.md`](.claude/rules/governance.md) 에
있으며, 4단계 PR rubric 으로 강제됩니다:

1. **구조적 불변 조건.** 추가 전용 워크플로우 원장의 불변 조건을
   강제하는가?
2. **범용성.** 요청자 외에 이 기능을 필요로 할 가상의 어답터 2명을
   말할 수 있는가?
3. **Opt-in 형태.** 커널 표면이 아닌 옵셔널 crate 로 배포 가능한가?
4. **Public-API 영향.** `docs/public_api/*.baseline` 이 바뀌는가?

경계선에 있는 기능은 옵셔널 Tier-5 crate 로 배포 (`knotch-frontmatter`,
`knotch-adr` 가 선례), 절대 커널 안에 넣지 않습니다.

### 무엇이 아닌가

- **태스크 트래커가 아닙니다.** unit 은 slug 기반 원장이지 티켓
  시스템이 아닙니다. 마일스톤은 커밋 trailer 로 명시적으로
  이름을 붙입니다 — knotch 는 free-form 메시지에서 마일스톤을
  임의로 만들어내지 않습니다.
- **시크릿 스캐너가 아닙니다.** 커밋 메시지는 로그에 그대로
  기록됩니다. `knotch hook check-commit` 앞단에 gitleaks / trufflehog /
  detect-secrets 를 걸어두세요. 스캐너가 설정되지 않으면
  `knotch doctor` 가 경고합니다.
- **문서 저장소가 아닙니다.** artifact 경로는 참조일 뿐이며, 실제
  파일은 프로젝트 레포지토리에 남습니다.
- **워크플로우 엔진이 아닙니다.** 대시보드 / 템플릿 카탈로그 /
  비즈니스 정책 / 프로젝트 브랜디드 룰이 명시적으로 veto 됩니다.
  Knotch 는 원장의 구조적 primitive 만 제공합니다.

---

## 품질 게이트

모든 push 에 대해 아래 항목이 실행되며 실패 시 merge 가 차단됩니다.

| 게이트 | 명령어 | 목적 |
|---|---|---|
| 포맷 | `cargo +nightly fmt --all --check` | nightly rustfmt 가 `rustfmt.toml` 의 unstable key (import grouping, comment wrap) 를 적용 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | stable + beta 툴체인에서 `-D warnings` |
| 테스트 | `cargo nextest run --workspace --all-features` + `cargo test --workspace --all-features --doc` | ubuntu / macos / windows × stable / beta, 그리고 MSRV 게이트를 겸하는 `ubuntu / 1.94 MSRV` 행 |
| 커버리지 | `cargo llvm-cov` | Codecov 로 업로드 |
| 구조 lint | `cargo knotch-linter` | R1 (DirectLogWriteRule), R2 (ForbiddenNameRule), R3 (KernelNoIoRule) |
| 미사용 dep | `cargo machete` | 워크스페이스 전체 |
| 보안 | `cargo deny check` | 라이선스 allowlist + CVE 권고 |
| Semver | `cargo semver-checks` | patch / minor / major 분류, 버전 범프 불일치 시 실패 |
| Public API | `cargo public-api --diff-against docs/public_api/<crate>.baseline` | 표면 변경 시 동일 commit 에서 baseline 갱신 필요 |
| 문서 인용 | `cargo xtask docs-lint` | `.claude/rules/` 의 `crate/path.rs:LINE` 인용이 여전히 resolve 되는지 |
| Fuzzing | `cargo fuzz` (nightly workflow) | 매일 예약 실행 |
| 설치 | `install-test.yml` | 3-OS × (from-source + prebuilt) 샌드박스 왕복 검증 |

`#![forbid(unsafe_code)]` 가 `Cargo.toml [workspace.lints.rust]` 에
워크스페이스 단위로 선언되어 있습니다. 예외는 없습니다 — 2026
safe-wrapper 스택 (`rustix`, `fs4`, `gix`) 이 모든 저수준 관심사를
커버합니다.

---

## 에이전트 통합

이 레포를 읽는 AI 에이전트라면 [`CLAUDE.md`](CLAUDE.md) 에서
시작하세요 — `.claude/rules/` 와 `.claude/skills/` 로 연결되는
progressive-disclosure 진입점입니다. 모든 주장은 `crate/path.rs:LINE`
인용으로 뒷받침되며 매 커밋마다 `cargo xtask docs-lint` 가 검증합니다.

- [`.claude/skills/knotch-query/SKILL.md`](.claude/skills/knotch-query/SKILL.md) — projection 읽기
- [`.claude/skills/knotch-mark/SKILL.md`](.claude/skills/knotch-mark/SKILL.md) — phase 완료 / 스킵 기록
- [`.claude/skills/knotch-gate/SKILL.md`](.claude/skills/knotch-gate/SKILL.md) — gate 결정 기록
- [`.claude/skills/knotch-transition/SKILL.md`](.claude/skills/knotch-transition/SKILL.md) — unit status 전환
- [`.claude/skills/knotch-approve/SKILL.md`](.claude/skills/knotch-approve/SKILL.md) — 사람이 이전 이벤트에 co-sign
- [`.claude/rules/hook-integration.md`](.claude/rules/hook-integration.md) — hook exit-code 계약
- [`.claude/rules/event-ownership.md`](.claude/rules/event-ownership.md) — variant 별 소유자 표

써드파티 harness 는 `knotch-cli` 를 래핑하기보다 `knotch-agent` 를
자체 바이너리에서 직접 임베드합니다. Hook 계약은 동일합니다 —
바이너리는 달라도 라이브러리는 같습니다.

자체 상태 레이어에서 마이그레이션하는 adopter 는
[`docs/migrations/README.md`](docs/migrations/README.md) 의 phased
패턴을 따릅니다. adopter-specific 계획은 해당 adopter 자신의
저장소에서 관리됩니다 — knotch 는 범용 playbook 만 제공하며
프로젝트 브랜드가 박힌 마이그레이션 문서는 싣지 않습니다.

---

## 라이선스

다음 중 하나로 dual license 됩니다:

- [Apache License, Version 2.0](./LICENSE-APACHE), 또는
- [MIT license](./LICENSE-MIT)

사용자 선택에 따라 둘 중 하나를 선택할 수 있습니다.

---

> **[English](README.md)** | **한국어**
