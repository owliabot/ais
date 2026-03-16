use ais_agent_control::execution_artifact::{ObserveStage, OutputExportSpec, TransactionStage};

use crate::{runtime::ActiveRun, runtime_value_resolver::resolve_value_ref};

fn export_outputs(
    runtime: &mut ActiveRun,
    exports: &[OutputExportSpec],
) -> Result<Vec<String>, String> {
    if runtime.checkpoint.execution_artifact.is_none() {
        return Err("execution_artifact runtime state is not present".to_owned());
    }

    let mut resolved = Vec::new();
    for export in exports {
        resolved.push((
            export.output_key.clone(),
            resolve_value_ref(runtime, &export.source)?,
        ));
    }

    let snapshot = runtime
        .checkpoint
        .execution_artifact
        .as_mut()
        .expect("checked is_some");
    let mut exported = Vec::new();
    for (output_key, value) in resolved {
        snapshot.exported_outputs.insert(output_key.clone(), value);
        exported.push(output_key.to_string());
    }
    Ok(exported)
}

pub(crate) fn export_transaction_outputs(
    runtime: &mut ActiveRun,
    stage: &TransactionStage,
) -> Result<Vec<String>, String> {
    export_outputs(runtime, &stage.exports)
}

pub(crate) fn export_observe_outputs(
    runtime: &mut ActiveRun,
    stage: &ObserveStage,
) -> Result<Vec<String>, String> {
    export_outputs(runtime, &stage.exports)
}
