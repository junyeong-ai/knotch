//! End-to-end QueryBuilder tests using InMemoryRepository.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use knotch_derive::MilestoneKind;
use knotch_kernel::{
    AppendMode, Causation, PhaseKind, Proposal, Repository, Scope, StatusId, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{CommitKind, CommitRef, EventBody, SkipKind},
};
use knotch_query::QueryBuilder;
use knotch_testing::InMemoryRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Ph {
    A,
    B,
}
impl PhaseKind for Ph {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Ph::A => "a",
            Ph::B => "b",
        })
    }
    fn is_skippable(&self, _: &SkipKind) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
pub enum Ms {
    Alpha,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum G {}
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Wf;
const PHASES: [Ph; 2] = [Ph::A, Ph::B];
impl WorkflowKind for Wf {
    type Phase = Ph;
    type Milestone = Ms;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("query-test")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
}

fn causation() -> Causation {
    Causation::new(Source::Cli, Principal::System { service: "test".into() }, Trigger::Manual)
}

fn p(body: EventBody<Wf>) -> Proposal<Wf> {
    Proposal { causation: causation(), extension: (), body, supersedes: None }
}

async fn seed_unit(repo: &InMemoryRepository<Wf>, id: &str, steps: Vec<EventBody<Wf>>) {
    let unit = UnitId::try_new(id).unwrap();
    let proposals: Vec<_> = steps.into_iter().map(p).collect();
    repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("seed");
}

#[tokio::test]
async fn where_phase_filters_units_at_that_phase() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_unit(&repo, "only-created", vec![EventBody::UnitCreated { scope: Scope::Standard }]).await;
    seed_unit(
        &repo,
        "past-a",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::PhaseCompleted { phase: Ph::A, artifacts: Default::default() },
        ],
    )
    .await;

    let units =
        QueryBuilder::<Wf>::new().where_phase(Ph::A).execute(&Wf, &repo).await.expect("execute");
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["only-created".to_owned()]
    );

    let units =
        QueryBuilder::<Wf>::new().where_phase(Ph::B).execute(&Wf, &repo).await.expect("execute");
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["past-a".to_owned()]
    );
}

#[tokio::test]
async fn where_milestone_shipped_matches_shipped_units() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_unit(
        &repo,
        "alpha-shipped",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::MilestoneShipped {
                milestone: Ms::Alpha,
                commit: CommitRef::new("abc"),
                commit_kind: CommitKind::Feat,
                status: knotch_kernel::CommitStatus::Verified,
            },
        ],
    )
    .await;
    seed_unit(&repo, "nothing", vec![EventBody::UnitCreated { scope: Scope::Standard }]).await;

    let units = QueryBuilder::<Wf>::new()
        .where_milestone_shipped(Ms::Alpha)
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].as_str(), "alpha-shipped");
}

#[tokio::test]
async fn where_status_matches_transitioned_units() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_unit(
        &repo,
        "in-review",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::StatusTransitioned {
                target: StatusId::new("in_review"),
                forced: false,
                rationale: None,
            },
        ],
    )
    .await;
    seed_unit(&repo, "draft", vec![EventBody::UnitCreated { scope: Scope::Standard }]).await;

    let units = QueryBuilder::<Wf>::new()
        .where_status(StatusId::new("in_review"))
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].as_str(), "in-review");
}

#[tokio::test]
async fn limit_caps_result_size() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    for i in 0..5 {
        seed_unit(
            &repo,
            &format!("u-{i}"),
            vec![EventBody::UnitCreated { scope: Scope::Standard }],
        )
        .await;
    }

    let units = QueryBuilder::<Wf>::new().limit(3).execute(&Wf, &repo).await.expect("execute");
    assert_eq!(units.len(), 3);
}

#[tokio::test]
async fn composed_filters_are_anded() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_unit(
        &repo,
        "match",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::MilestoneShipped {
                milestone: Ms::Alpha,
                commit: CommitRef::new("abc"),
                commit_kind: CommitKind::Feat,
                status: knotch_kernel::CommitStatus::Verified,
            },
            EventBody::StatusTransitioned {
                target: StatusId::new("in_review"),
                forced: false,
                rationale: None,
            },
        ],
    )
    .await;
    seed_unit(
        &repo,
        "no-milestone",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::StatusTransitioned {
                target: StatusId::new("in_review"),
                forced: false,
                rationale: None,
            },
        ],
    )
    .await;

    let units = Arc::new(
        QueryBuilder::<Wf>::new()
            .where_milestone_shipped(Ms::Alpha)
            .where_status(StatusId::new("in_review"))
            .execute(&Wf, &repo)
            .await
            .expect("execute"),
    );
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].as_str(), "match");
}

// --- P1-3 cross-adapter + group-by battery --------------------------------

async fn seed_adapter<R>(repo: &R, id: &str, steps: Vec<EventBody<Wf>>)
where
    R: knotch_kernel::Repository<Wf>,
{
    let unit = UnitId::try_new(id).unwrap();
    let proposals: Vec<_> = steps.into_iter().map(p).collect();
    repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("seed");
}

#[tokio::test]
async fn query_result_parity_between_in_memory_and_file_backed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let memory = InMemoryRepository::<Wf>::new(Wf);
    let file = knotch_storage::FileRepository::<Wf>::new(dir.path(), Wf);

    let seed = async |name: &'static str, body: EventBody<Wf>| {
        seed_adapter(
            &memory,
            name,
            vec![EventBody::UnitCreated { scope: Scope::Standard }, body.clone()],
        )
        .await;
        seed_adapter(&file, name, vec![EventBody::UnitCreated { scope: Scope::Standard }, body])
            .await;
    };
    seed("phase-a", EventBody::PhaseCompleted { phase: Ph::A, artifacts: Default::default() })
        .await;
    seed("phase-b", EventBody::PhaseCompleted { phase: Ph::A, artifacts: Default::default() })
        .await;
    seed(
        "in-review",
        EventBody::StatusTransitioned {
            target: StatusId::new("in_review"),
            forced: false,
            rationale: None,
        },
    )
    .await;

    let mem_at_b =
        QueryBuilder::<Wf>::new().where_phase(Ph::B).execute(&Wf, &memory).await.expect("mem");
    let file_at_b =
        QueryBuilder::<Wf>::new().where_phase(Ph::B).execute(&Wf, &file).await.expect("file");
    let mut mem_ids: Vec<_> = mem_at_b.iter().map(|u| u.as_str().to_owned()).collect();
    let mut file_ids: Vec<_> = file_at_b.iter().map(|u| u.as_str().to_owned()).collect();
    mem_ids.sort();
    file_ids.sort();
    assert_eq!(mem_ids, file_ids, "where_phase results diverged across adapters");

    let mem_in_review = QueryBuilder::<Wf>::new()
        .where_status(StatusId::new("in_review"))
        .execute(&Wf, &memory)
        .await
        .expect("mem");
    let file_in_review = QueryBuilder::<Wf>::new()
        .where_status(StatusId::new("in_review"))
        .execute(&Wf, &file)
        .await
        .expect("file");
    assert_eq!(mem_in_review.len(), file_in_review.len());
    assert_eq!(
        mem_in_review.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        file_in_review.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        "where_status results diverged across adapters",
    );
}

/// Group-by-phase over the InMemoryRepository — mirrors the charter's
/// "iterate units, project their `current_phase`, group by phase"
/// composition sketch.
#[tokio::test]
async fn group_units_by_current_phase() {
    use std::collections::BTreeMap;

    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_unit(&repo, "u1", vec![EventBody::UnitCreated { scope: Scope::Standard }]).await;
    seed_unit(
        &repo,
        "u2",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::PhaseCompleted { phase: Ph::A, artifacts: Default::default() },
        ],
    )
    .await;
    seed_unit(
        &repo,
        "u3",
        vec![
            EventBody::UnitCreated { scope: Scope::Standard },
            EventBody::PhaseCompleted { phase: Ph::A, artifacts: Default::default() },
        ],
    )
    .await;

    // Group via list_units + load + current_phase — this is the
    // exact pattern a dashboard composition sketch uses.
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    use futures::StreamExt as _;
    let mut stream = Repository::list_units(&repo);
    while let Some(slot) = stream.next().await {
        let unit = slot.expect("list_units");
        let log = repo.load(&unit).await.expect("load");
        let phase = knotch_kernel::project::current_phase(&Wf, &log);
        let key = phase
            .as_ref()
            .map(|p| PhaseKind::id(p).to_string())
            .unwrap_or_else(|| "(none)".to_string());
        groups.entry(key).or_default().push(unit.as_str().to_owned());
    }

    // u1 has no phase yet → first pending phase is `a`.
    // u2/u3 completed `a` → first pending phase is `b`.
    let mut at_a = groups.remove("a").unwrap_or_default();
    at_a.sort();
    let mut at_b = groups.remove("b").unwrap_or_default();
    at_b.sort();
    assert_eq!(at_a, vec!["u1".to_owned()]);
    assert_eq!(at_b, vec!["u2".to_owned(), "u3".to_owned()]);
}

// ---- causation predicates ---------------------------------------------

use compact_str::CompactString;
use knotch_kernel::causation::{AgentId, Cost, Harness, ModelId};

fn agent_causation(agent: &str, model: &str, harness: &str) -> Causation {
    Causation::new(
        Source::Agent,
        Principal::Agent {
            agent_id: AgentId(CompactString::from(agent)),
            model: ModelId(CompactString::from(model)),
            harness: Harness(CompactString::from(harness)),
        },
        Trigger::ToolInvocation {
            tool: CompactString::from("test-tool"),
            call_id: CompactString::from("call-1"),
        },
    )
}

fn agent_causation_with_cost(agent: &str, model: &str, harness: &str, usd_cents: i64) -> Causation {
    let mut c = agent_causation(agent, model, harness);
    c = c.with_cost(Cost::new(
        Some(rust_decimal::Decimal::new(usd_cents, 2)),
        100,
        200,
    ));
    c
}

async fn seed_with_causation(
    repo: &InMemoryRepository<Wf>,
    id: &str,
    causation: Causation,
    body: EventBody<Wf>,
) {
    let unit = UnitId::try_new(id).unwrap();
    let proposal = Proposal { causation, extension: (), body, supersedes: None };
    repo.append(&unit, vec![proposal], AppendMode::BestEffort).await.expect("seed");
}

#[tokio::test]
async fn where_agent_id_filters_to_matching_events() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_with_causation(
        &repo,
        "by-alice",
        agent_causation("alice", "opus", "claude-code"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;
    seed_with_causation(
        &repo,
        "by-bob",
        agent_causation("bob", "opus", "claude-code"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;

    let units = QueryBuilder::<Wf>::new()
        .where_agent_id(AgentId(CompactString::from("alice")))
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["by-alice".to_owned()]
    );
}

#[tokio::test]
async fn where_model_partitions_by_llm() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_with_causation(
        &repo,
        "opus-unit",
        agent_causation("a", "claude-opus-4-7", "claude-code"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;
    seed_with_causation(
        &repo,
        "haiku-unit",
        agent_causation("a", "claude-haiku-4-5", "claude-code"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;

    let units = QueryBuilder::<Wf>::new()
        .where_model(ModelId(CompactString::from("claude-opus-4-7")))
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["opus-unit".to_owned()]
    );
}

#[tokio::test]
async fn where_harness_separates_cohorts() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    seed_with_causation(
        &repo,
        "cc-unit",
        agent_causation("a", "opus", "claude-code"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;
    seed_with_causation(
        &repo,
        "cursor-unit",
        agent_causation("a", "opus", "cursor"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;

    let units = QueryBuilder::<Wf>::new()
        .where_harness(Harness(CompactString::from("cursor")))
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["cursor-unit".to_owned()]
    );
}

#[tokio::test]
async fn where_cost_gte_includes_only_units_above_bound() {
    let repo = InMemoryRepository::<Wf>::new(Wf);
    // unit-cheap: $0.10 (10 cents). unit-spendy: $5.00. unit-unknown: no cost.
    seed_with_causation(
        &repo,
        "unit-cheap",
        agent_causation_with_cost("a", "m", "h", 10),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;
    seed_with_causation(
        &repo,
        "unit-spendy",
        agent_causation_with_cost("a", "m", "h", 500),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;
    seed_with_causation(
        &repo,
        "unit-unknown",
        agent_causation("a", "m", "h"),
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await;

    let units = QueryBuilder::<Wf>::new()
        .where_cost_gte(rust_decimal::Decimal::new(100, 2)) // $1.00
        .execute(&Wf, &repo)
        .await
        .expect("execute");
    // Only unit-spendy clears the $1 bar. unit-cheap under. unit-unknown
    // opts out — None != 0 per constitution §VIII comment in
    // `.claude/rules/causation.md`.
    assert_eq!(
        units.iter().map(|u| u.as_str().to_owned()).collect::<Vec<_>>(),
        vec!["unit-spendy".to_owned()]
    );
}
