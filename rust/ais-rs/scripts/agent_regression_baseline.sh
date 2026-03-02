#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/agent_regression_baseline.sh [--group <name>]

Run ais-runner agent critical regression tests by exact test name.
Any failed test exits immediately.

Groups:
  all           Run all groups (default)
  cli           Agent CLI parsing/guardrails
  segmented     Segmented intent core loop
  safety        Safety gates and state persistence
  wave1         Wave-1 regression guardrails (P1-121/P2-200/P2-220-prep)
  wave2         Wave-2 regression guardrails (P1-122/P2-210)
  wave3         Wave-3 regression guardrails (AGT-P1-123/AGT-P2-220-main)
  wave4         Wave-4 regression guardrails (AGT-P1-124/AGT-P2-230)
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

GROUP="all"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --group)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --group" >&2
        usage
        exit 2
      fi
      GROUP="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

CLI_TESTS=(
  "cli::tests::cli_parses_agent_command"
  "cli::tests::cli_parses_agent_intent_command"
  "cli::tests::cli_parses_agent_intent_file_command"
  "cli::tests::cli_rejects_agent_with_both_plan_and_intent"
  "cli::tests::cli_rejects_agent_with_both_intent_and_intent_file"
  "cli::tests::cli_rejects_agent_without_plan_or_intent"
)

SEGMENTED_TESTS=(
  "agent::tests::execute_agent_intent_mode_requires_workspace_candidates"
  "agent::tests::execute_agent_intent_mode_requires_llm_provider_with_workspace"
  "agent::tests::execute_agent_segmented_missing_required_input_pauses_instead_of_failing"
  "agent::tests::execute_agent_segmented_intent_fixture_queries_then_pauses_for_confirm"
  "agent::tests::segmented_intent_fixture_revise_with_until_retry_then_complete"
  "agent::tests::segmented_intent_fixture_repairs_format_then_compiles_assert_branch_segment"
  "agent::tests::planner_invalid_output_error_is_retryable"
)

SAFETY_TESTS=(
  "agent::tests::engine_stays_paused_when_user_denies_confirmation"
  "agent::tests::write_gate_validation_rejects_transfer_without_assert_branch_chain"
  "agent::tests::write_gate_validation_requires_token_decimals_when_asset_input_lacks_decimals"
  "agent::tests::write_gate_validation_rejects_stale_volatile_facts_without_refresh_query"
  "agent::tests::write_gate_validation_accepts_refresh_query_for_volatile_facts"
  "agent::tests::checkpoint_extensions_roundtrip_restores_fact_store_todo_and_intent_facts"
  "agent::tests::load_or_init_state_rejects_legacy_checkpoint_node_ids"
  "agent::tests::segmented_checkpoint_resume_keeps_need_user_confirm_pause_in_real_flow"
  "agent::tests::apply_missing_input_answers_backfills_runtime_and_fact_store"
  "agent::tests::apply_missing_input_answers_normalizes_inputs_prefixed_keys"
  "run::tests::checkpoint_resume_keeps_need_user_confirm_decision_stable"
  "run::tests::checkpoint_side_effect_sent_prevents_tx_replay_after_restart"
)

WAVE1_TESTS=(
  "agent::tests::execute_agent_segmented_missing_required_input_pauses_instead_of_failing"
  "agent::tests::record_todo_progress_tracks_follow_up_todo_after_completion"
  "agent::tests::todo_phase_error_payload_namespaces_reason_and_round"
  "agent::tests::state_summary_projects_input_registry_missing_slots_from_todo_and_questions"
  "agent::tests::state_summary_includes_node_output_refs_projection_for_segment_outputs"
  "agent::tests::planner_invalid_output_payload_has_stable_reason_code"
  "agent::tests::planner_missing_candidate_ref_payload_has_targeted_sub_reason_and_hint"
  "agent::tests::planner_invalid_status_payload_has_expected_hint_contract"
  "agent::tests::compile_error_state_payload_classifies_unknown_input_ref_issue"
)

WAVE2_TESTS=(
  "agent::tests::segment_planning_phase_migration_keeps_planner_error_contract_parity"
  "agent::tests::phase_machine_plan_segment_transition_contract_stays_stable"
  "agent::tests::context_envelope_keeps_projected_summary_contract_compatible"
  "agent::tests::context_envelope_hash_and_unchanged_flags_track_payload_mutations"
)

WAVE3_TESTS=(
  "agent::tests::execute_segment_phase_migration_keeps_pause_and_fail_contract_parity"
  "agent::tests::typed_context_core_path_switch_keeps_projection_and_envelope_contract_parity"
)

WAVE4_TESTS=(
  "agent::tests::execute_segment_phase_migration_keeps_pause_and_fail_contract_parity"
  "agent::tests::resolve_pause_backflow_preserves_missing_input_and_confirm_split"
  "agent::tests::execution_pause_payload_uses_compatible_reason_subreason_codes"
  "agent::tests::planner_error_payload_subreason_enum_is_backward_compatible"
)

run_group() {
  local group_name="$1"
  shift
  local tests=("$@")

  echo
  echo "==> [${group_name}]"
  for test_name in "${tests[@]}"; do
    echo "--> ${test_name}"
    cargo test -p ais-runner "${test_name}" -- --exact
  done
}

cd "${WORKSPACE_ROOT}"

case "${GROUP}" in
  all)
    run_group "cli" "${CLI_TESTS[@]}"
    run_group "segmented" "${SEGMENTED_TESTS[@]}"
    run_group "safety" "${SAFETY_TESTS[@]}"
    run_group "wave1" "${WAVE1_TESTS[@]}"
    run_group "wave2" "${WAVE2_TESTS[@]}"
    run_group "wave3" "${WAVE3_TESTS[@]}"
    run_group "wave4" "${WAVE4_TESTS[@]}"
    ;;
  cli)
    run_group "cli" "${CLI_TESTS[@]}"
    ;;
  segmented)
    run_group "segmented" "${SEGMENTED_TESTS[@]}"
    ;;
  safety)
    run_group "safety" "${SAFETY_TESTS[@]}"
    ;;
  wave1)
    run_group "wave1" "${WAVE1_TESTS[@]}"
    ;;
  wave2)
    run_group "wave2" "${WAVE2_TESTS[@]}"
    ;;
  wave3)
    run_group "wave3" "${WAVE3_TESTS[@]}"
    ;;
  wave4)
    run_group "wave4" "${WAVE4_TESTS[@]}"
    ;;
  *)
    echo "Invalid group: ${GROUP}" >&2
    usage
    exit 2
    ;;
esac

echo
echo "Agent regression baseline finished successfully."
