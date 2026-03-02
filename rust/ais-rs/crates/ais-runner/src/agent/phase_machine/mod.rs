use super::RunnerError;

pub(super) mod grounding;
pub(super) mod pause;
pub(super) mod segment_exec;
pub(super) mod segment_plan;
pub(super) mod todo;
pub(super) mod types;

use types::{AgentPhase, PhaseTransition};

#[derive(Debug, Clone)]
pub(super) struct PhaseMachineRunner {
    current_phase: AgentPhase,
    transitions: Vec<PhaseTransition>,
}

impl PhaseMachineRunner {
    pub(super) fn new() -> Self {
        Self {
            current_phase: AgentPhase::Init,
            transitions: Vec::new(),
        }
    }

    pub(super) fn current_phase(&self) -> AgentPhase {
        self.current_phase
    }

    pub(super) fn transitions(&self) -> &[PhaseTransition] {
        self.transitions.as_slice()
    }

    pub(super) fn apply_transition(&mut self, transition: PhaseTransition) {
        self.current_phase = transition.next_phase();
        self.transitions.push(transition);
    }

    fn bootstrap_main_flow(&mut self) {
        self.apply_transition(PhaseTransition::Advance {
            from: AgentPhase::Init,
            to: AgentPhase::GroundIntent,
        });
        self.apply_transition(PhaseTransition::Stay {
            phase: AgentPhase::GroundIntent,
            reason: "segmented_main_flow",
        });
    }
}

pub(super) struct MainFlowPhaseTracker<'a> {
    runner: &'a mut PhaseMachineRunner,
    verbose: bool,
}

impl MainFlowPhaseTracker<'_> {
    fn new(runner: &mut PhaseMachineRunner, verbose: bool) -> MainFlowPhaseTracker<'_> {
        MainFlowPhaseTracker { runner, verbose }
    }

    pub(super) fn transition_to(&mut self, phase: AgentPhase, reason: &'static str) {
        let previous = self.runner.current_phase();
        if previous == phase {
            self.runner
                .apply_transition(PhaseTransition::Stay { phase, reason });
            if self.verbose {
                eprintln!(
                    "[agent.phase_machine] transition kind=stay phase={} reason={} transitions={}",
                    phase.as_str(),
                    reason,
                    self.runner.transitions().len()
                );
            }
            return;
        }

        self.runner.apply_transition(PhaseTransition::Advance {
            from: previous,
            to: phase,
        });
        if self.verbose {
            eprintln!(
                "[agent.phase_machine] transition kind=advance from={} to={} reason={} transitions={}",
                previous.as_str(),
                phase.as_str(),
                reason,
                self.runner.transitions().len()
            );
        }
    }
}

fn run_main_flow_internal<F>(
    verbose: bool,
    run_main: F,
) -> (Result<String, RunnerError>, PhaseMachineRunner)
where
    F: FnOnce(&mut MainFlowPhaseTracker<'_>) -> Result<String, RunnerError>,
{
    let mut runner = PhaseMachineRunner::new();
    runner.bootstrap_main_flow();
    if verbose {
        eprintln!(
            "[agent.phase_machine] main_flow_enter phase={} transitions={}",
            runner.current_phase().as_str(),
            runner.transitions().len()
        );
    }

    let run_result = {
        let mut phase_tracker = MainFlowPhaseTracker::new(&mut runner, verbose);
        run_main(&mut phase_tracker)
    };

    let result = match run_result {
        Ok(output) => {
            runner.apply_transition(PhaseTransition::Complete);
            if verbose {
                eprintln!(
                    "[agent.phase_machine] main_flow_exit status=completed phase={} transitions={}",
                    runner.current_phase().as_str(),
                    runner.transitions().len()
                );
            }
            Ok(output)
        }
        Err(error) => {
            let fail_phase = runner.current_phase();
            runner.apply_transition(PhaseTransition::Fail {
                phase: fail_phase,
                reason: "segmented_main_flow_failed".to_string(),
            });
            if verbose {
                eprintln!(
                    "[agent.phase_machine] main_flow_exit status=failed fail_phase={} terminal_phase={} transitions={} error={}",
                    fail_phase.as_str(),
                    runner.current_phase().as_str(),
                    runner.transitions().len(),
                    error
                );
            }
            Err(error)
        }
    };

    (result, runner)
}

pub(super) fn run_main_flow<F>(verbose: bool, run_main: F) -> Result<String, RunnerError>
where
    F: FnOnce(&mut MainFlowPhaseTracker<'_>) -> Result<String, RunnerError>,
{
    run_main_flow_internal(verbose, run_main).0
}

#[cfg(test)]
#[path = "../tests/phase_machine/mod.rs"]
mod tests;
